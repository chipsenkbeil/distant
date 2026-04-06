//! Mount test infrastructure for integration tests.
//!
//! Provides [`MountProcess`] for managing foreground mount lifecycles,
//! [`wait_for_unmount`] for polling unmount completion, singleton mount
//! coordination via [`get_or_start_mount`], and macOS FileProvider `.app`
//! bundle installation. Also re-exports [`MountBackend`] for convenience
//! and defines rstest-reuse templates for parameterized mount tests.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use rstest::rstest;
#[allow(unused_imports)]
use rstest_reuse::{self, *};

pub use distant_mount::MountBackend;

#[allow(unused_imports)]
use crate::backend::Backend;
use crate::backend::BackendCtx;

/// Timeout for polling `mount` command output until the mount point disappears.
const UNMOUNT_POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between `mount` command polls in [`wait_for_unmount`].
const UNMOUNT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Maximum time to wait for `distant unmount` CLI during test cleanup.
const DROP_UNMOUNT_TIMEOUT: Duration = Duration::from_secs(15);

/// Interval for polling `distant unmount` CLI exit status.
const DROP_UNMOUNT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A managed mount process for integration tests.
///
/// Wraps a child process running `distant mount --foreground` and handles
/// cleanup (unmount via manager + directory removal) on drop.
pub struct MountProcess {
    /// Mount ID returned by the manager.
    mount_id: Option<u32>,
    /// Socket path for sending unmount commands.
    socket_or_pipe: String,
    mount_point: PathBuf,

    /// When true, this mount was created via the macOS FileProvider backend.
    #[allow(dead_code)]
    is_file_provider: bool,

    /// Path to the `.app` bundle binary used for FileProvider mounts.
    #[allow(dead_code)]
    /// Stored so that Drop can invoke `unmount --all` through the same binary.
    #[cfg(target_os = "macos")]
    bundled_bin: Option<PathBuf>,
}

impl MountProcess {
    /// Spawns a `distant mount` process and waits for it to be ready.
    ///
    /// For FUSE/NFS/WCF backends, uses `ctx.new_std_cmd(["mount"])` to get a
    /// properly configured command, then adds `--backend`, `--foreground`, any
    /// extra `args`, and the `mount_point`. Blocks until the process prints
    /// "Mounted" on stdout (up to 30 seconds). The mount point is then
    /// canonicalized to resolve macOS `/var` to `/private/var` symlinks.
    ///
    /// For macOS FileProvider, builds a `.app` bundle via
    /// [`build_test_app_bundle`] and runs the bundled binary with the context's
    /// socket. The process exits after printing "Mounted" and the mount point
    /// is discovered under `~/Library/CloudStorage/`.
    ///
    /// Returns `Err` if the mount fails to start (process exits early or
    /// doesn't print "Mounted" within the timeout). The caller can use this
    /// to test error cases without leaking processes.
    pub fn try_spawn(
        ctx: &BackendCtx,
        mount: MountBackend,
        mount_point: &Path,
        args: &[&str],
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        if matches!(mount, MountBackend::MacosFileProvider) {
            return Self::try_spawn_file_provider(ctx, args);
        }

        Self::try_spawn_foreground(ctx, mount, mount_point, args)
    }

    /// Mounts via the manager. The CLI sends a mount request to the manager,
    /// prints the result, and exits immediately.
    fn try_spawn_foreground(
        ctx: &BackendCtx,
        mount: MountBackend,
        mount_point: &Path,
        args: &[&str],
    ) -> Result<Self, String> {
        if let Err(e) = std::fs::create_dir_all(mount_point) {
            return Err(format!(
                "failed to create mount point {}: {e}",
                mount_point.display()
            ));
        }

        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.arg("--backend").arg(mount.as_str());

        for arg in args {
            cmd.arg(arg);
        }

        cmd.arg(mount_point);

        let output = cmd
            .output()
            .map_err(|e| format!("failed to run distant mount: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mount failed: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse mount ID from output: "Mounted at /path (id: 123)"
        let mount_id = stdout
            .lines()
            .find(|l| l.contains("Mounted"))
            .and_then(|l| l.rsplit("id: ").next())
            .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

        let canonical = std::fs::canonicalize(mount_point).map_err(|e| {
            format!(
                "failed to canonicalize mount point {}: {e}",
                mount_point.display()
            )
        })?;

        Ok(Self {
            mount_id,
            socket_or_pipe: ctx.socket_or_pipe().to_string(),
            mount_point: canonical,
            is_file_provider: false,
            #[cfg(target_os = "macos")]
            bundled_bin: None,
        })
    }

    /// Mounts via the FileProvider backend using the installed `.app` binary.
    ///
    /// The command sends a mount request through the FP singleton's manager,
    /// prints the result, and exits immediately. The mount point is parsed
    /// from the CLI stdout.
    #[cfg(target_os = "macos")]
    fn try_spawn_file_provider(_ctx: &BackendCtx, args: &[&str]) -> Result<Self, String> {
        use crate::{manager, singleton};

        let fp_handle = singleton::get_or_start_file_provider();

        let bin = PathBuf::from("/Applications/Distant.app/Contents/MacOS/distant");

        let mut cmd = Command::new(&bin);
        cmd.arg("mount");
        cmd.arg("--log-file")
            .arg(manager::random_log_file("mount-fp"));
        cmd.arg("--log-level").arg("trace");
        cmd.arg("--unix-socket").arg(&fp_handle.socket_or_pipe);
        cmd.arg("--backend").arg("macos-file-provider");
        cmd.args(["--extra", "poll_interval=0.05"]);

        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("failed to run distant mount (file-provider): {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("file-provider mount failed:\nstderr: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mount_id = stdout
            .lines()
            .find(|l| l.contains("Mounted"))
            .and_then(|l| l.rsplit("id: ").next())
            .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

        // Parse mount point from "Mounted at /path (id: 123)"
        let mount_point_str = stdout
            .lines()
            .find(|l| l.starts_with("Mounted at "))
            .and_then(|l| l.strip_prefix("Mounted at "))
            .and_then(|l| l.rsplit_once(" (id: ").map(|(path, _)| path))
            .ok_or("mount output did not contain mount point")?;
        let mount_point = PathBuf::from(mount_point_str);
        wait_for_fp_mount_ready(&mount_point);

        Ok(Self {
            mount_id,
            socket_or_pipe: fp_handle.socket_or_pipe.clone(),
            mount_point,
            is_file_provider: true,
            #[cfg(target_os = "macos")]
            bundled_bin: Some(bin),
        })
    }

    /// Spawns a `distant mount --foreground` process and waits for it to be ready.
    ///
    /// # Panics
    ///
    /// Panics if the mount fails to start. Use [`try_spawn`](Self::try_spawn)
    /// for tests that expect the mount to fail.
    pub fn spawn(ctx: &BackendCtx, mount: MountBackend, mount_point: &Path, args: &[&str]) -> Self {
        Self::try_spawn(ctx, mount, mount_point, args)
            .unwrap_or_else(|e| panic!("mount failed: {e}"))
    }

    /// Returns the canonicalized mount point path.
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    /// Returns the mount ID assigned by the manager, if available.
    pub fn mount_id(&self) -> Option<u32> {
        self.mount_id
    }

    /// Returns the manager socket (Unix) or pipe (Windows) path.
    pub fn socket_or_pipe(&self) -> &str {
        &self.socket_or_pipe
    }
}

impl Drop for MountProcess {
    fn drop(&mut self) {
        // Unmount via manager if we have a mount ID.
        // The CLI waits for the manager's Unmounted response, so by the
        // time this returns the mount should already be fully removed.
        let unmount_ok = if let Some(id) = self.mount_id {
            let mut cmd = Command::new(crate::manager::bin_path());
            cmd.arg("unmount").arg(id.to_string());

            if cfg!(windows) {
                cmd.arg("--windows-pipe").arg(&self.socket_or_pipe);
            } else {
                cmd.arg("--unix-socket").arg(&self.socket_or_pipe);
            }

            cmd.stdout(Stdio::null()).stderr(Stdio::null());

            // Use spawn + poll instead of blocking .status() so we can
            // time out if the manager hangs.
            match cmd.spawn() {
                Ok(mut child) => {
                    let start = Instant::now();
                    loop {
                        match child.try_wait() {
                            Ok(Some(status)) => break status.success(),
                            Ok(None) if start.elapsed() > DROP_UNMOUNT_TIMEOUT => {
                                eprintln!("[MountProcess::drop] unmount {id} timed out, killing");
                                let _ = child.kill();
                                break false;
                            }
                            Ok(None) => std::thread::sleep(DROP_UNMOUNT_POLL_INTERVAL),
                            Err(_) => break false,
                        }
                    }
                }
                Err(_) => false,
            }
        } else {
            false
        };

        if !unmount_ok {
            // Safety net: force unmount via OS if the manager unmount failed
            #[cfg(target_os = "macos")]
            {
                let mp = self.mount_point.to_string_lossy();
                let _ = Command::new("diskutil")
                    .args(["unmount", "force", &mp])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }

            #[cfg(all(unix, not(target_os = "macos")))]
            {
                let mp = self.mount_point.to_string_lossy();
                let _ = Command::new("umount")
                    .arg("-f")
                    .arg(&*mp)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }

            wait_for_unmount(&self.mount_point);
        }

        let _ = std::fs::remove_dir_all(&self.mount_point);
    }
}

/// Polls the system `mount` command until the given path no longer appears
/// in its output, or until a 5-second timeout elapses.
///
/// This is useful after issuing an unmount command to wait for the kernel
/// to fully release the mount point before attempting directory cleanup.
pub fn wait_for_unmount(mount_point: &Path) {
    let mp_str = mount_point.to_string_lossy();
    let start = Instant::now();

    while start.elapsed() < UNMOUNT_POLL_TIMEOUT {
        let output = Command::new("mount").output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                if !text.contains(&*mp_str) {
                    return;
                }
            }
            Err(_) => return,
        }
        std::thread::sleep(UNMOUNT_POLL_INTERVAL);
    }
}

/// Waits for a FileProvider mount point to become accessible.
///
/// After `addDomain`, macOS needs time to launch the appex, bootstrap it,
/// and run the initial enumeration. The CloudStorage directory exists
/// immediately but `read_dir` may error until the appex responds.
/// Polls every 200ms for up to 30s.
#[cfg(target_os = "macos")]
fn wait_for_fp_mount_ready(mount_point: &Path) {
    let start = Instant::now();
    let timeout = Duration::from_secs(30);
    let poll = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if std::fs::read_dir(mount_point).is_ok() {
            return;
        }
        std::thread::sleep(poll);
    }
    eprintln!(
        "warning: FP mount point {} not accessible after {timeout:?}",
        mount_point.display()
    );
}

/// Waits for a path inside an FP mount to become visible locally.
///
/// FileProvider refreshes directory listings periodically (controlled by
/// `poll_interval` in MountConfig.extra). This helper polls the local path
/// until it exists, giving macOS time to process the enumerator refresh.
pub fn wait_for_fp_path(path: &Path) {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let poll = Duration::from_millis(100);

    while start.elapsed() < timeout {
        if path.exists() {
            return;
        }
        std::thread::sleep(poll);
    }
    // Don't panic — let the test assertion handle the failure message
}

/// Poll until a condition is met on the remote, or timeout after 10 seconds.
///
/// Calls `check` every 200ms. Returns `Ok(())` when `check` returns `true`,
/// or panics with `msg` if the timeout expires.
fn poll_until(check: impl Fn() -> bool, msg: &str) {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let interval = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if check() {
            return;
        }
        std::thread::sleep(interval);
    }
    panic!("poll timeout after {}s: {msg}", timeout.as_secs());
}

/// Poll until a remote file exists, or panic after 10 seconds.
pub fn wait_until_exists(ctx: &BackendCtx, path: &str) {
    poll_until(
        || ctx.cli_exists(path),
        &format!("waiting for {path} to exist"),
    );
}

/// Poll until a remote file has the expected content, or panic after 10 seconds.
pub fn wait_until_content(ctx: &BackendCtx, path: &str, expected: &str) {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let interval = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if ctx.cli_exists(path) && ctx.cli_read(path) == expected {
            return;
        }
        std::thread::sleep(interval);
    }

    let actual = if ctx.cli_exists(path) {
        ctx.cli_read(path)
    } else {
        "<file does not exist>".to_string()
    };
    panic!(
        "poll timeout after {}s: waiting for {path} to contain {expected:?}, actual: {actual:?}",
        timeout.as_secs()
    );
}

/// Poll until a remote path no longer exists, or panic after 10 seconds.
pub fn wait_until_gone(ctx: &BackendCtx, path: &str) {
    poll_until(
        || !ctx.cli_exists(path),
        &format!("waiting for {path} to disappear"),
    );
}

/// Deprecated: use [`wait_until_exists`], [`wait_until_content`], or
/// [`wait_until_gone`] instead for polling-based sync verification.
pub fn wait_for_sync() {
    std::thread::sleep(Duration::from_secs(2));
}

/// Removes all registered FileProvider domains by shelling out to the
/// installed `.app` binary. FileProvider APIs require bundle context,
/// so this cannot be called directly from the test process.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn cleanup_stale_fp_domains() {
    let bin = std::path::PathBuf::from("/Applications/Distant.app/Contents/MacOS/distant");
    if !bin.exists() {
        return;
    }
    eprintln!("[cleanup] removing stale FileProvider domains via .app binary");
    match Command::new(&bin)
        .args(["unmount", "--include-all-macos-file-provider-domains"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() {
                eprintln!("[cleanup] {}", stdout.trim());
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "[cleanup] warning: FP domain cleanup failed: {}",
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!("[cleanup] warning: failed to run FP domain cleanup: {e}");
        }
    }
}

/// Force-unmount all stale distant mounts (NFS + FUSE) and poll until
/// the OS mount table is clear. Call before asserting "no mounts found".
pub fn cleanup_all_stale_mounts() {
    // Clean up stale FileProvider domains on macOS. Each domain creates a
    // CloudStorage entry that persists across process restarts. Without
    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let output = match Command::new("mount").stdout(Stdio::piped()).output() {
                Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                Err(_) => return,
            };

            let stale: Vec<String> = output
                .lines()
                .filter(|line| {
                    let is_nfs = line.contains("localhost:/") && line.contains("nfs");
                    let is_fuse = line.starts_with("distant ") || line.contains("FSName=distant");
                    is_nfs || is_fuse
                })
                .filter_map(|line| {
                    line.split(" on ")
                        .nth(1)
                        .and_then(|rest| rest.split(" (").next())
                        .map(|s| s.to_string())
                })
                .collect();

            if stale.is_empty() {
                return;
            }

            for path in &stale {
                let _ = Command::new("umount").arg("-f").arg(path).output();
                #[cfg(target_os = "macos")]
                {
                    let _ = Command::new("diskutil")
                        .args(["unmount", "force"])
                        .arg(path)
                        .output();
                }
            }

            if Instant::now() >= deadline {
                eprintln!("warning: stale mounts still present after 10s: {stale:?}");
                return;
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Installs the test `.app` bundle to `/Applications/Distant.app`.
///
/// Backs up any existing production install to `/Applications/Distant.app.bak`.
/// Uses `scripts/build-macos-app.sh --skip-build` to bundle, sign, and install.
/// Requires an Apple Development signing identity in the keychain.
///
/// # Errors
///
/// Returns `Err` if signing identity is missing, the script fails, or
/// `/Applications/` is not writable.
#[cfg(target_os = "macos")]
pub fn install_test_app() -> Result<(), String> {
    let workspace = find_workspace_root();
    let app = PathBuf::from("/Applications/Distant.app");
    let app_bin = app.join("Contents").join("MacOS").join("distant");
    let backup = PathBuf::from("/Applications/Distant.app.bak");

    // Skip if the installed app binary matches the build output (same size
    // and not older). Mtime alone is insufficient because cargo build and
    // the install script can run in the same second, producing equal mtimes
    // for different binaries.
    let build_bin = crate::manager::build_dir().join("distant");
    if app_bin.exists() && build_bin.exists() {
        let app_meta = app_bin.metadata().ok();
        let build_meta = build_bin.metadata().ok();
        if let (Some(app_m), Some(build_m)) = (app_meta, build_meta) {
            let same_size = app_m.len() == build_m.len();
            let app_newer = app_m
                .modified()
                .and_then(|a| build_m.modified().map(|b| a >= b))
                .unwrap_or(false);
            if same_size && app_newer {
                eprintln!("[install_test_app] app is up-to-date, skipping install");
                return Ok(());
            }
        }
    }

    // Back up existing production install (only if not already backed up)
    if app.exists() && !backup.exists() {
        std::fs::rename(&app, &backup)
            .map_err(|e| format!("failed to back up /Applications/Distant.app: {e}"))?;
    }

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    eprintln!(
        "[install_test_app] installing app from {}",
        build_bin.display()
    );
    let status = Command::new("bash")
        .arg(workspace.join("scripts/build-macos-app.sh"))
        .arg("--skip-build")
        .env("CARGO_PROFILE", profile)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| format!("failed to run build-macos-app.sh: {e}"))?;

    if !status.success() {
        // Restore backup on failure
        restore_production_app();
        return Err(
            "build-macos-app.sh failed (missing Apple Development signing identity?)".into(),
        );
    }

    Ok(())
}

/// Restores the production `/Applications/Distant.app` from backup.
///
/// Called after tests complete or if install fails. Re-registers the
/// restored extension with pluginkit.
#[cfg(target_os = "macos")]
pub fn restore_production_app() {
    let app = PathBuf::from("/Applications/Distant.app");
    let backup = PathBuf::from("/Applications/Distant.app.bak");

    let _ = std::fs::remove_dir_all(&app);

    if backup.exists() {
        let _ = std::fs::rename(&backup, &app);
        // Re-register the restored extension
        let appex = app
            .join("Contents")
            .join("PlugIns")
            .join("DistantFileProvider.appex");
        let _ = Command::new("pluginkit")
            .args(["-a", &appex.to_string_lossy()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pluginkit")
            .args(["-e", "use", "-i", "dev.distant.file-provider"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Template for testing all plugin backends (host, ssh, docker).
#[template]
#[export]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[cfg_attr(feature = "docker", case::docker(Backend::Docker))]
pub fn all_plugins(#[case] backend: Backend) {}

/// Template for testing all combinations of plugin backends and mount backends.
#[template]
#[export]
#[rstest]
#[cfg_attr(
    feature = "mount-nfs",
    case::host_nfs(Backend::Host, MountBackend::Nfs)
)]
#[cfg_attr(feature = "mount-nfs", case::ssh_nfs(Backend::Ssh, MountBackend::Nfs))]
#[cfg_attr(
    all(feature = "docker", feature = "mount-nfs"),
    case::docker_nfs(Backend::Docker, MountBackend::Nfs)
)]
#[cfg_attr(
    all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ),
    case::host_fuse(Backend::Host, MountBackend::Fuse)
)]
#[cfg_attr(
    all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ),
    case::ssh_fuse(Backend::Ssh, MountBackend::Fuse)
)]
#[cfg_attr(
    all(feature = "mount-windows-cloud-files", target_os = "windows"),
    case::host_wcf(Backend::Host, MountBackend::WindowsCloudFiles)
)]
#[cfg_attr(
    all(feature = "mount-macos-file-provider", target_os = "macos"),
    case::host_fp(Backend::Host, MountBackend::MacosFileProvider)
)]
pub fn plugin_x_mount(#[case] backend: Backend, #[case] mount: MountBackend) {}

/// Metadata stored in the mount singleton meta file.
///
/// Persisted as JSON so that concurrent test processes can discover and
/// reuse an existing mount without re-creating it.
#[derive(Debug, Serialize, Deserialize)]
struct MountMeta {
    /// Mount ID returned by the manager, if available.
    mount_id: Option<u32>,
    /// Local mount point path.
    mount_point: PathBuf,
    /// Remote root directory that the mount exposes.
    remote_root: String,
}

/// Handle to a shared singleton mount.
///
/// The caller must keep this alive for the duration of the test to
/// maintain the shared file lock, signaling that a client is still
/// using the mount.
pub struct MountSingletonHandle {
    /// Local mount point path.
    pub mount_point: PathBuf,
    /// Remote root directory that the mount exposes.
    pub remote_root: String,
    /// Shared lock file handle — held (not read) so the lock is released on drop.
    #[allow(dead_code)]
    lock_file: File,
    /// Holds the FP server singleton alive for FileProvider mounts.
    /// Each test must hold this to prevent the FP manager's lonely
    /// timeout from firing between tests.
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    fp_handle: Option<crate::singleton::SingletonHandle>,
}

impl MountSingletonHandle {
    /// Builds a command routed through the correct manager for this mount.
    ///
    /// For FileProvider mounts, uses the FP App Group socket. For all other
    /// backends, delegates to `ctx.new_std_cmd`.
    pub fn new_std_cmd(
        &self,
        ctx: &BackendCtx,
        subcommands: impl IntoIterator<Item = &'static str>,
    ) -> Command {
        #[cfg(target_os = "macos")]
        if let Some(ref fp) = self.fp_handle {
            let mut cmd = Command::new(crate::manager::bin_path());
            for sub in subcommands {
                cmd.arg(sub);
            }
            cmd.arg("--log-file")
                .arg(crate::manager::random_log_file("client-fp"))
                .arg("--log-level")
                .arg("trace")
                .arg("--unix-socket")
                .arg(&fp.socket_or_pipe);
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            return cmd;
        }

        ctx.new_std_cmd(subcommands)
    }
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

/// Returns the string key for a plugin backend variant.
fn backend_name(ctx: &BackendCtx) -> &'static str {
    match ctx.backend() {
        Backend::Host => "host",
        Backend::Ssh => "ssh",
        #[cfg(feature = "docker")]
        Backend::Docker => "docker",
    }
}

/// Returns the base path for a mount singleton, without extension.
fn mount_base_path(backend: &str, mount: &str) -> PathBuf {
    let hash = workspace_hash();
    std::env::temp_dir().join(format!("distant-test-{hash}-mount-{backend}-{mount}"))
}

/// Returns the path to the lock file for a mount singleton.
fn mount_lock_path(backend: &str, mount: &str) -> PathBuf {
    let mut p = mount_base_path(backend, mount);
    p.set_extension("lock");
    p
}

/// Returns the path to the meta (JSON) file for a mount singleton.
fn mount_meta_path(backend: &str, mount: &str) -> PathBuf {
    let mut p = mount_base_path(backend, mount);
    p.set_extension("meta");
    p
}

/// Reads and validates the meta file for a mount singleton.
///
/// Returns `None` if the meta file is missing, unparseable, or the mount
/// point is no longer an active mount (indicating a stale singleton).
fn read_live_mount_meta(backend: &str, mount: &str) -> Option<MountMeta> {
    let path = mount_meta_path(backend, mount);
    let content = fs::read_to_string(&path).ok()?;
    let meta: MountMeta = serde_json::from_str(&content).ok()?;

    // FileProvider mounts appear under ~/Library/CloudStorage/, not in
    // the OS `mount` table. Check directory existence instead.
    let is_fp = mount == MountBackend::MacosFileProvider.as_str();
    let alive = if is_fp {
        meta.mount_point.is_dir()
    } else {
        is_mount_active(&meta.mount_point)
    };

    if alive {
        Some(meta)
    } else {
        eprintln!(
            "[mount-singleton] stale meta for {backend}/{mount}: \
             mount point {} is not an active mount, cleaning up",
            meta.mount_point.display()
        );
        let _ = fs::remove_file(&path);
        None
    }
}

/// Checks whether a path is an active mount point by looking for it in
/// the OS `mount` command output.
fn is_mount_active(path: &Path) -> bool {
    let output = Command::new("mount").output().ok();
    match output {
        Some(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains(&*path.to_string_lossy())
        }
        None => false,
    }
}

/// Creates a new mount via the CLI.
///
/// For macOS FileProvider mounts, uses the installed `.app` binary and the
/// FP singleton's socket. For all other backends, uses
/// [`BackendCtx::new_std_cmd`].
fn start_mount(
    ctx: &BackendCtx,
    mount: MountBackend,
    mount_point: &Path,
    remote_root: &str,
) -> MountMeta {
    #[cfg(target_os = "macos")]
    if matches!(mount, MountBackend::MacosFileProvider) {
        return start_file_provider_mount(mount_point, remote_root);
    }

    start_foreground_mount(ctx, mount, mount_point, remote_root)
}

/// Creates a mount via the manager for non-FileProvider backends.
fn start_foreground_mount(
    ctx: &BackendCtx,
    mount: MountBackend,
    mount_point: &Path,
    remote_root: &str,
) -> MountMeta {
    if let Err(e) = fs::create_dir_all(mount_point) {
        panic!(
            "failed to create mount point {}: {e}",
            mount_point.display()
        );
    }

    let mut cmd = ctx.new_std_cmd(["mount"]);
    cmd.arg("--backend")
        .arg(mount.as_str())
        .arg("--remote-root")
        .arg(remote_root)
        .arg(mount_point);

    eprintln!("[mount-singleton] creating mount: {cmd:?}");
    let output = cmd.output().expect("failed to run distant mount");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("[mount-singleton] mount failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse mount ID from output: "Mounted at /path (id: 123)"
    let mount_id = stdout
        .lines()
        .find(|l| l.contains("Mounted"))
        .and_then(|l| l.rsplit("id: ").next())
        .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

    MountMeta {
        mount_id,
        mount_point: mount_point.to_path_buf(),
        remote_root: remote_root.to_string(),
    }
}

/// Creates a mount via the macOS FileProvider backend.
///
/// Uses the installed `.app` binary and the FP singleton's socket to send
/// the mount request. The mount point is parsed from the CLI stdout.
#[cfg(target_os = "macos")]
fn start_file_provider_mount(_mount_point: &Path, remote_root: &str) -> MountMeta {
    use crate::{manager, singleton};

    let fp_handle = singleton::get_or_start_file_provider();

    let bin = PathBuf::from("/Applications/Distant.app/Contents/MacOS/distant");

    let mut cmd = Command::new(&bin);
    cmd.arg("mount");
    cmd.arg("--log-file")
        .arg(manager::random_log_file("mount-singleton-fp"));
    cmd.arg("--log-level").arg("trace");
    cmd.arg("--unix-socket").arg(&fp_handle.socket_or_pipe);
    cmd.arg("--backend").arg("macos-file-provider");
    cmd.args(["--extra", "poll_interval=0.05"]);
    cmd.arg("--remote-root").arg(remote_root);

    eprintln!("[mount-singleton] creating file-provider mount: {cmd:?}");
    let output = cmd
        .output()
        .expect("failed to run distant mount (file-provider)");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("[mount-singleton] file-provider mount failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mount_id = stdout
        .lines()
        .find(|l| l.contains("Mounted"))
        .and_then(|l| l.rsplit("id: ").next())
        .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

    // Parse mount point from "Mounted at /path (id: 123)"
    let mount_point_str = stdout
        .lines()
        .find(|l| l.starts_with("Mounted at "))
        .and_then(|l| l.strip_prefix("Mounted at "))
        .and_then(|l| l.rsplit_once(" (id: ").map(|(path, _)| path))
        .expect("[mount-singleton] mount output did not contain mount point");
    let mount_point = PathBuf::from(mount_point_str);
    wait_for_fp_mount_ready(&mount_point);

    // Keep the FP handle alive so the lock is not released. Since this
    // is a singleton mount, leak it — the manager will self-terminate
    // via --shutdown lonely=N.
    std::mem::forget(fp_handle);

    MountMeta {
        mount_id,
        mount_point,
        remote_root: remote_root.to_string(),
    }
}

/// Gets or creates a singleton mount for the given backend and mount type.
///
/// Uses file-lock coordination so the first test process to run creates
/// the mount, and subsequent processes reuse it. The caller **must** keep
/// the returned [`MountSingletonHandle`] alive for the duration of the
/// test to maintain the shared lock.
///
/// # Panics
///
/// Panics if the mount fails to start or if file-lock operations fail.
pub fn get_or_start_mount(ctx: &BackendCtx, mount: MountBackend) -> MountSingletonHandle {
    let backend = backend_name(ctx);
    let mount_str = mount.as_str();

    let lp = mount_lock_path(backend, mount_str);
    let mp = mount_meta_path(backend, mount_str);

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

    let meta = if let Some(meta) = read_live_mount_meta(backend, mount_str) {
        eprintln!(
            "[mount-singleton] reusing existing {backend}/{mount_str} mount at {}",
            meta.mount_point.display()
        );
        meta
    } else {
        eprintln!("[mount-singleton] starting new {backend}/{mount_str} mount");

        // Create remote root via the CLI
        let remote_root = ctx.unique_dir(&format!("mount-shared-{mount_str}"));
        ctx.cli_mkdir(&remote_root);

        // Create local mount point
        let hash = workspace_hash();
        let mount_point =
            std::env::temp_dir().join(format!("distant-mount-{hash}-{backend}-{mount_str}"));

        let meta = start_mount(ctx, mount, &mount_point, &remote_root);

        let content = serde_json::to_string_pretty(&meta).expect("failed to serialize mount meta");
        fs::write(&mp, content).expect("failed to write mount meta");

        meta
    };

    // Downgrade to shared lock — other test processes can now read the meta
    // and join as additional clients
    lock_file
        .lock_shared()
        .expect("failed to downgrade to shared lock");

    // For FileProvider mounts, each test must hold the FP server singleton
    // handle to keep the FP manager alive. Without this, the lonely timeout
    // fires between tests and the FP manager shuts down.
    #[cfg(target_os = "macos")]
    let fp_handle = if matches!(mount, MountBackend::MacosFileProvider) {
        Some(crate::singleton::get_or_start_file_provider())
    } else {
        None
    };

    MountSingletonHandle {
        mount_point: meta.mount_point,
        remote_root: meta.remote_root,
        lock_file,
        #[cfg(target_os = "macos")]
        fp_handle,
    }
}

/// Creates a unique subdirectory under `parent` via the CLI.
///
/// Returns `(full_remote_path, subdir_name)`. The directory is created
/// immediately through the distant CLI so it exists on the remote before
/// returning.
pub fn unique_subdir(ctx: &BackendCtx, parent: &str, label: &str) -> (String, String) {
    let name = format!("{label}-{}", rand::random::<u64>());
    let path = ctx.child_path(parent, &name);
    ctx.cli_mkdir(&path);
    (path, name)
}
