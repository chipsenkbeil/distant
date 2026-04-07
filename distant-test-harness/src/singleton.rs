//! Singleton test server infrastructure.
//!
//! Provides shared server instances across nextest test processes using
//! file-lock coordination. The first test to run starts the servers;
//! subsequent tests reuse them. Servers auto-exit via `--shutdown lonely=10`
//! after the last client disconnects.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use crate::manager;
use crate::process;
use crate::sshd;

use distant_core::Credentials;
use distant_core::net::common::Host;
use std::io::BufReader;
use std::net::{Ipv4Addr, Ipv6Addr};

/// How long to wait for the server to become idle before auto-shutdown.
const LONELY_TIMEOUT_SECS: u32 = 30;

/// Max retries for connecting manager to server.
const MAX_CONNECT_RETRIES: usize = 3;

/// Metadata persisted to disk for each singleton server.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerMeta {
    /// PID of the manager process.
    pub manager_pid: u32,
    /// PID of the server process (host backend only).
    pub server_pid: Option<u32>,
    /// PID of the sshd process (SSH backend only).
    pub sshd_pid: Option<u32>,
    /// Port the sshd is listening on (SSH backend only).
    pub sshd_port: Option<u16>,
    /// Path to the sshd temporary directory containing keys and config.
    pub sshd_dir: Option<String>,
    /// Unix socket path or Windows named pipe for the manager.
    pub socket_path: String,
    /// Docker container ID (reserved for future use).
    pub container_id: Option<String>,
}

/// Result of [`get_or_start_host`] or [`get_or_start_ssh`].
///
/// Contains the socket path for connecting to the manager and a lock file
/// handle. The caller must keep the handle alive for the duration of the
/// test to maintain the shared lock — this signals that a client is still
/// using the singleton.
pub struct SingletonHandle {
    /// Unix socket path or Windows named pipe for the manager.
    pub socket_or_pipe: String,
    /// Shared lock file handle. Dropping this releases the shared lock.
    pub lock_file: File,
}

/// Returns a truncated hash of the workspace root for namespacing temp files.
fn workspace_hash() -> String {
    let root = find_workspace_root();
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

/// Finds the workspace root by walking up from `CARGO_MANIFEST_DIR`.
fn find_workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists()
            && let Ok(content) = fs::read_to_string(&cargo_toml)
            && content.contains("[workspace]")
        {
            return dir;
        }
        if !dir.pop() {
            panic!(
                "could not find workspace root from {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

/// Returns the base path for a given backend, without extension.
fn base_path(backend: &str) -> PathBuf {
    let hash = workspace_hash();
    std::env::temp_dir().join(format!("distant-test-{hash}-{backend}"))
}

/// Returns the path to the lock file for a given backend.
fn lock_path(backend: &str) -> PathBuf {
    let mut p = base_path(backend);
    p.set_extension("lock");
    p
}

/// Returns the path to the meta (JSON) file for a given backend.
fn meta_path(backend: &str) -> PathBuf {
    let mut p = base_path(backend);
    p.set_extension("meta");
    p
}

/// Returns the path to the socket file for a given backend.
fn sock_path(backend: &str) -> PathBuf {
    let mut p = base_path(backend);
    p.set_extension("sock");
    p
}

/// Checks if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: Sending signal 0 does not actually deliver a signal; it only
        // performs error checking (permission and existence).
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On Windows, probe via tasklist. A zero-exit means the PID exists.
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // tasklist prints "INFO: No tasks..." when the PID is absent
                o.status.success() && !stdout.contains("No tasks")
            })
            .unwrap_or(false)
    }
}

/// Sends SIGKILL (Unix) or taskkill (Windows) to a process.
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: Simple signal-sending syscall with a valid PID.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Reads and validates the meta file for a backend.
///
/// Returns `None` if the meta file is missing, unparseable, or references
/// a dead manager process. Stale meta files are cleaned up automatically.
fn read_live_meta(backend: &str) -> Option<ServerMeta> {
    let path = meta_path(backend);
    let content = fs::read_to_string(&path).ok()?;
    let meta: ServerMeta = serde_json::from_str(&content).ok()?;

    if is_pid_alive(meta.manager_pid) {
        Some(meta)
    } else {
        eprintln!(
            "[singleton] stale meta for {backend}: manager PID {} is dead, cleaning up",
            meta.manager_pid
        );
        cleanup_meta(&meta);
        let _ = fs::remove_file(&path);
        None
    }
}

/// Kills all processes referenced by a meta, removes the socket file,
/// and force-removes any Docker container.
fn cleanup_meta(meta: &ServerMeta) {
    kill_pid(meta.manager_pid);
    if let Some(pid) = meta.server_pid {
        kill_pid(pid);
    }
    if let Some(pid) = meta.sshd_pid {
        kill_pid(pid);
    }
    if let Some(ref name) = meta.container_id {
        let _ = Command::new("docker")
            .args(["rm", "-f", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = fs::remove_file(&meta.socket_path);
}

/// Starts a singleton Host (local) backend.
///
/// Spawns a manager and a server, connects them, and returns metadata
/// describing the running processes.
fn start_host(socket_path: &Path) -> ServerMeta {
    let socket_str = socket_path.to_string_lossy().to_string();

    let mut manager_cmd = Command::new(manager::bin_path());
    manager_cmd
        .arg("manager")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-manager"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"));

    if cfg!(windows) {
        manager_cmd.arg("--windows-pipe").arg(&socket_str);
    } else {
        manager_cmd.arg("--unix-socket").arg(&socket_str);
    }

    manager_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut manager_cmd);

    eprintln!("[singleton] starting host manager: {manager_cmd:?}");
    let mut mgr = manager_cmd
        .spawn()
        .expect("failed to spawn singleton manager");
    let manager_pid = mgr.id();

    manager::wait_for_manager_ready(&socket_str, &mut mgr);

    // Release handles so the detached process doesn't hold our pipe
    let _ = mgr.stdout.take();
    let _ = mgr.stderr.take();

    // Start server
    let mut server_cmd = Command::new(manager::bin_path());
    server_cmd
        .arg("server")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-server"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut server_cmd);

    eprintln!("[singleton] starting host server: {server_cmd:?}");
    let mut server = server_cmd
        .spawn()
        .expect("failed to spawn singleton server");
    let server_pid = server.id();

    let credentials = read_server_credentials(&mut server);

    connect_manager_to_server(&socket_str, &credentials);

    // Leak both processes so they outlive this test process.
    // The manager will self-terminate via --shutdown lonely=N.
    std::mem::forget(mgr);
    std::mem::forget(server);

    ServerMeta {
        manager_pid,
        server_pid: Some(server_pid),
        sshd_pid: None,
        sshd_port: None,
        sshd_dir: None,
        socket_path: socket_str,
        container_id: None,
    }
}

/// Starts a singleton SSH backend.
///
/// Spawns a manager and an sshd, connects the manager to the sshd via
/// `distant connect ssh://...`, and returns metadata describing the
/// running processes.
fn start_ssh(socket_path: &Path) -> ServerMeta {
    let socket_str = socket_path.to_string_lossy().to_string();

    // Start manager
    let mut manager_cmd = Command::new(manager::bin_path());
    manager_cmd
        .arg("manager")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-ssh-manager"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"));

    if cfg!(windows) {
        manager_cmd.arg("--windows-pipe").arg(&socket_str);
    } else {
        manager_cmd.arg("--unix-socket").arg(&socket_str);
    }

    manager_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut manager_cmd);

    eprintln!("[singleton] starting ssh manager: {manager_cmd:?}");
    let mut mgr = manager_cmd
        .spawn()
        .expect("failed to spawn ssh singleton manager");
    let manager_pid = mgr.id();
    manager::wait_for_manager_ready(&socket_str, &mut mgr);
    let _ = mgr.stdout.take();
    let _ = mgr.stderr.take();

    // Spawn sshd
    let sshd = sshd::Sshd::spawn(Default::default()).expect("failed to spawn singleton sshd");
    let sshd_port = sshd.port;
    let sshd_dir = sshd.tmp.path().to_string_lossy().to_string();

    // Extract the sshd child PID before we leak the Sshd
    let sshd_pid = sshd
        .child
        .lock()
        // Safety: the mutex is not poisoned — we just spawned the sshd
        .unwrap()
        .as_ref()
        .map(|c| c.id());

    // Build SSH connect options
    let ssh_options = format!(
        "identity_files={dir}/id_ed25519,user_known_hosts_files={dir}/known_hosts,identities_only=true",
        dir = sshd_dir,
    );
    let destination = format!("ssh://{}@127.0.0.1:{}", *sshd::USERNAME, sshd_port);

    let mut connected = false;
    for i in 1..=MAX_CONNECT_RETRIES {
        let mut connect_cmd = Command::new(manager::bin_path());
        connect_cmd
            .arg("connect")
            .arg("--log-file")
            .arg(manager::random_log_file("singleton-ssh-connect"))
            .arg("--log-level")
            .arg("trace")
            .arg("--options")
            .arg(&ssh_options);

        if cfg!(windows) {
            connect_cmd.arg("--windows-pipe").arg(&socket_str);
        } else {
            connect_cmd.arg("--unix-socket").arg(&socket_str);
        }

        connect_cmd.arg(&destination);
        connect_cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        eprintln!("[singleton] ssh connect attempt {i}/{MAX_CONNECT_RETRIES}: {connect_cmd:?}");
        let output = connect_cmd.output().expect("failed to run connect");

        if output.status.success() {
            connected = true;
            break;
        }

        eprintln!(
            "[singleton] ssh connect attempt {i} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        thread::sleep(Duration::from_millis(500));
    }

    if !connected {
        panic!(
            "[singleton] failed to connect manager to SSH server after {MAX_CONNECT_RETRIES} attempts"
        );
    }

    // Leak manager and sshd so they outlive the test process.
    std::mem::forget(mgr);
    std::mem::forget(sshd);

    ServerMeta {
        manager_pid,
        server_pid: None,
        sshd_pid,
        sshd_port: Some(sshd_port),
        sshd_dir: Some(sshd_dir),
        socket_path: socket_str,
        container_id: None,
    }
}

/// Starts a singleton FileProvider backend.
///
/// Installs the test app to `/Applications/Distant.app`, then starts a
/// manager + server using the installed binary. The manager listens on the
/// App Group container socket so the FileProvider extension can find it.
#[cfg(all(target_os = "macos", feature = "mount"))]
fn start_file_provider(_socket_path: &Path) -> ServerMeta {
    // Install the test app (idempotent — skips rebuild if mtime matches)
    crate::mount::install_test_app().expect("failed to install test app for FileProvider");

    let app_bin = PathBuf::from("/Applications/Distant.app/Contents/MacOS/distant");
    let home = std::env::var("HOME").expect("HOME not set");
    let group_dir = format!("{home}/Library/Group Containers/39C6AGD73Z.group.dev.distant");
    let app_group_socket = format!("{group_dir}/distant.sock");

    // Remove stale socket from previous runs
    let _ = fs::remove_file(&app_group_socket);
    let _ = fs::create_dir_all(&group_dir);

    // Start manager using the INSTALLED binary (so it detects the app bundle
    // and creates the socket in the App Group container).
    let mut manager_cmd = Command::new(&app_bin);
    manager_cmd
        .arg("manager")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-fp-manager"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"))
        .arg("--unix-socket")
        .arg(&app_group_socket);

    manager_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut manager_cmd);

    eprintln!("[singleton] starting file-provider manager: {manager_cmd:?}");
    let mut mgr = manager_cmd
        .spawn()
        .expect("failed to spawn file-provider singleton manager");
    let manager_pid = mgr.id();

    manager::wait_for_manager_ready(&app_group_socket, &mut mgr);

    let _ = mgr.stdout.take();
    let _ = mgr.stderr.take();

    // Start server (using the installed binary so it's consistent)
    let mut server_cmd = Command::new(&app_bin);
    server_cmd
        .arg("server")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-fp-server"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut server_cmd);

    eprintln!("[singleton] starting file-provider server: {server_cmd:?}");
    let mut server = server_cmd
        .spawn()
        .expect("failed to spawn file-provider singleton server");
    let server_pid = server.id();

    let credentials = read_server_credentials(&mut server);
    connect_manager_to_server(&app_group_socket, &credentials);

    // Leak processes so they outlive the test process
    std::mem::forget(mgr);
    std::mem::forget(server);

    ServerMeta {
        manager_pid,
        server_pid: Some(server_pid),
        sshd_pid: None,
        sshd_port: None,
        sshd_dir: None,
        socket_path: app_group_socket,
        container_id: None,
    }
}

/// Starts a singleton Docker backend.
///
/// Creates a Docker container, spawns a manager, and connects the manager
/// to the container via `distant connect docker://`. Returns `None` if
/// Docker is not available.
#[cfg(feature = "docker")]
fn start_docker(socket_path: &Path) -> Option<ServerMeta> {
    use crate::docker;

    // Create the container on a background thread with its own Tokio runtime
    // to avoid nesting runtimes.
    let container = std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create runtime for Docker container");
        rt.block_on(docker::DockerContainer::new())
    })
    .join()
    .expect("Docker container creation thread panicked")?;

    let container_name = container.name.clone();
    let socket_str = socket_path.to_string_lossy().to_string();

    // Start manager
    let mut manager_cmd = Command::new(manager::bin_path());
    manager_cmd
        .arg("manager")
        .arg("listen")
        .arg("--log-file")
        .arg(manager::random_log_file("singleton-docker-manager"))
        .arg("--log-level")
        .arg("trace")
        .arg("--shutdown")
        .arg(format!("lonely={LONELY_TIMEOUT_SECS}"));

    if cfg!(windows) {
        manager_cmd.arg("--windows-pipe").arg(&socket_str);
    } else {
        manager_cmd.arg("--unix-socket").arg(&socket_str);
    }

    manager_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::set_process_group(&mut manager_cmd);

    eprintln!("[singleton] starting docker manager: {manager_cmd:?}");
    let mut mgr = manager_cmd
        .spawn()
        .expect("failed to spawn docker singleton manager");
    let manager_pid = mgr.id();
    manager::wait_for_manager_ready(&socket_str, &mut mgr);
    let _ = mgr.stdout.take();
    let _ = mgr.stderr.take();

    // Connect manager to the Docker container
    let destination = format!("docker://{container_name}");
    let mut connected = false;
    for i in 1..=MAX_CONNECT_RETRIES {
        let mut connect_cmd = Command::new(manager::bin_path());
        connect_cmd
            .arg("connect")
            .arg("--log-file")
            .arg(manager::random_log_file("singleton-docker-connect"))
            .arg("--log-level")
            .arg("trace");

        if cfg!(windows) {
            connect_cmd.arg("--windows-pipe").arg(&socket_str);
        } else {
            connect_cmd.arg("--unix-socket").arg(&socket_str);
        }

        connect_cmd.arg(&destination);
        connect_cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        eprintln!("[singleton] docker connect attempt {i}/{MAX_CONNECT_RETRIES}: {connect_cmd:?}");
        let output = connect_cmd.output().expect("failed to run connect");

        if output.status.success() {
            connected = true;
            break;
        }

        eprintln!(
            "[singleton] docker connect attempt {i} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        thread::sleep(Duration::from_millis(500));
    }

    if !connected {
        eprintln!(
            "[singleton] failed to connect manager to Docker container after {MAX_CONNECT_RETRIES} attempts"
        );
        kill_pid(manager_pid);
        // Drop the container to clean it up
        drop(container);
        return None;
    }

    // Leak both manager and container so they outlive this test process.
    // The manager will self-terminate via --shutdown lonely=N.
    // The container keeps running via `sleep infinity`.
    std::mem::forget(mgr);
    std::mem::forget(container);

    Some(ServerMeta {
        manager_pid,
        server_pid: None,
        sshd_pid: None,
        sshd_port: None,
        sshd_dir: None,
        socket_path: socket_str,
        container_id: Some(container_name),
    })
}

/// Checks if a Docker container with the given name is running.
#[cfg(feature = "docker")]
fn is_container_alive(name: &str) -> bool {
    Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", name])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Reads [`Credentials`] from a server's stdout.
///
/// Spawns a background thread to read from the child's stdout, looking for
/// the credentials string. Panics if credentials are not found within 5
/// seconds.
fn read_server_credentials(server: &mut Child) -> Credentials {
    let stdout = server.stdout.take().expect("server stdout not piped");
    let handle = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut lines = String::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            lines.push_str(&String::from_utf8_lossy(&buf[..n]));
            if let Some(creds) = Credentials::find(&lines, false) {
                return creds;
            }
        }
        panic!("[singleton] failed to read server credentials from stdout");
    });

    let start = Instant::now();
    while !handle.is_finished() {
        if start.elapsed() > Duration::from_secs(5) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    handle.join().expect("credential reader panicked")
}

/// Connects the manager at `socket` to a server using `credentials`.
///
/// Tries multiple host representations (IPv4, IPv6, hostname string) to
/// account for platform-specific listener behavior.
fn connect_manager_to_server(socket: &str, credentials: &Credentials) {
    for host in [
        Host::Ipv4(Ipv4Addr::LOCALHOST),
        Host::Ipv6(Ipv6Addr::LOCALHOST),
        Host::Name("127.0.0.1".to_string()),
    ] {
        let mut creds = credentials.clone();
        creds.host = host.clone();

        let mut cmd = Command::new(manager::bin_path());
        cmd.arg("connect")
            .arg("--log-file")
            .arg(manager::random_log_file("singleton-connect"))
            .arg("--log-level")
            .arg("trace");

        if cfg!(windows) {
            cmd.arg("--windows-pipe").arg(socket);
        } else {
            cmd.arg("--unix-socket").arg(socket);
        }

        cmd.arg(creds.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        eprintln!("[singleton] connecting manager to server (host={host}): {cmd:?}");
        let output = cmd.output().expect("failed to run connect");

        if output.status.success() {
            eprintln!("[singleton] connected successfully via {host}");
            return;
        }
    }

    panic!("[singleton] failed to connect manager to server on any host variant");
}

/// Gets or starts a singleton Host backend.
///
/// Returns a handle containing the socket path and a shared file lock.
/// The caller **must** keep the [`SingletonHandle`] alive for the duration
/// of the test to maintain the shared lock.
pub fn get_or_start_host() -> SingletonHandle {
    get_or_start("host", start_host)
}

/// Gets or starts a singleton SSH backend.
///
/// Returns a handle containing the socket path and a shared file lock.
/// The caller **must** keep the [`SingletonHandle`] alive for the duration
/// of the test to maintain the shared lock.
pub fn get_or_start_ssh() -> SingletonHandle {
    get_or_start("ssh", start_ssh)
}

/// Gets or starts a singleton FileProvider backend.
///
/// Installs the test app to `/Applications/Distant.app`, starts a manager
/// that listens on the App Group container socket, and connects a local
/// server. The FileProvider extension connects to this manager via the
/// standard App Group socket path.
#[cfg(all(target_os = "macos", feature = "mount"))]
pub fn get_or_start_file_provider() -> SingletonHandle {
    get_or_start("file-provider", start_file_provider)
}

/// Gets or starts a singleton Docker backend.
///
/// Returns `None` if Docker is not available (no daemon, not Linux engine,
/// etc.). On success returns a handle and the container name, which the
/// caller needs to address the container in Docker CLI commands.
///
/// The caller **must** keep the [`SingletonHandle`] alive for the duration
/// of the test to maintain the shared lock.
#[cfg(feature = "docker")]
pub fn get_or_start_docker() -> Option<(SingletonHandle, String)> {
    let backend = "docker";
    let lp = lock_path(backend);
    let mp = meta_path(backend);
    let sp = sock_path(backend);

    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lp)
        .unwrap_or_else(|e| panic!("failed to open lock file {}: {e}", lp.display()));

    lock_file
        .lock_exclusive()
        .expect("failed to acquire exclusive lock");

    let (socket_or_pipe, container_name) = if let Some(meta) = read_live_meta(backend) {
        // Verify the container is still running
        let cid = meta.container_id.as_deref().unwrap_or("");
        if cid.is_empty() || !is_container_alive(cid) {
            eprintln!(
                "[singleton] stale docker meta: container '{}' is gone, cleaning up",
                cid
            );
            cleanup_meta(&meta);
            let _ = fs::remove_file(meta_path(backend));

            match start_docker(&sp) {
                Some(meta) => {
                    let socket = meta.socket_path.clone();
                    let name = meta
                        .container_id
                        .clone()
                        .expect("start_docker sets container_id");
                    let content =
                        serde_json::to_string_pretty(&meta).expect("failed to serialize meta");
                    fs::write(&mp, content).expect("failed to write meta");
                    (socket, name)
                }
                None => {
                    lock_file
                        .lock_shared()
                        .expect("failed to downgrade to shared lock");
                    return None;
                }
            }
        } else {
            eprintln!(
                "[singleton] reusing existing docker server (manager PID {}, container '{}')",
                meta.manager_pid, cid
            );
            (meta.socket_path, cid.to_string())
        }
    } else {
        eprintln!("[singleton] starting new docker server");
        match start_docker(&sp) {
            Some(meta) => {
                let socket = meta.socket_path.clone();
                let name = meta
                    .container_id
                    .clone()
                    .expect("start_docker sets container_id");
                let content =
                    serde_json::to_string_pretty(&meta).expect("failed to serialize meta");
                fs::write(&mp, content).expect("failed to write meta");
                (socket, name)
            }
            None => {
                lock_file
                    .lock_shared()
                    .expect("failed to downgrade to shared lock");
                return None;
            }
        }
    };

    lock_file
        .lock_shared()
        .expect("failed to downgrade to shared lock");

    Some((
        SingletonHandle {
            socket_or_pipe,
            lock_file,
        },
        container_name,
    ))
}

/// Core get-or-start logic shared between backends.
///
/// Acquires an exclusive file lock, checks for a live server via the meta
/// file, starts one if needed, then downgrades to a shared lock before
/// returning.
fn get_or_start(backend: &str, starter: fn(&Path) -> ServerMeta) -> SingletonHandle {
    let lp = lock_path(backend);
    let mp = meta_path(backend);
    let sp = sock_path(backend);

    // Open/create lock file
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lp)
        .unwrap_or_else(|e| panic!("failed to open lock file {}: {e}", lp.display()));

    // Exclusive lock for the startup check
    lock_file
        .lock_exclusive()
        .expect("failed to acquire exclusive lock");

    let socket_or_pipe = if let Some(meta) = read_live_meta(backend) {
        eprintln!(
            "[singleton] reusing existing {backend} server (manager PID {})",
            meta.manager_pid
        );
        meta.socket_path
    } else {
        eprintln!("[singleton] starting new {backend} server");
        let meta = starter(&sp);
        let socket = meta.socket_path.clone();

        let content = serde_json::to_string_pretty(&meta).expect("failed to serialize meta");
        fs::write(&mp, content).expect("failed to write meta");

        socket
    };

    // Downgrade to shared lock — other test processes can now read the meta
    // and join as additional clients
    lock_file
        .lock_shared()
        .expect("failed to downgrade to shared lock");

    SingletonHandle {
        socket_or_pipe,
        lock_file,
    }
}
