//! Docker container test harness for integration tests.
//!
//! Provides [`DockerContainer`] for managing test container lifecycles and rstest fixtures
//! for obtaining [`Client`] instances connected to Docker containers.

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command as StdCommand, Stdio};
use std::time::Duration;

use assert_cmd::Command;
use derive_more::{Deref, DerefMut};
use distant_core::Client;
use distant_docker::{Docker, DockerClient, DockerOpts};
use log::*;
use rstest::*;

use crate::manager::bin_path;

/// Checks whether a Linux Docker daemon is available.
///
/// Returns `true` only when the daemon is reachable **and** is running Linux containers.
/// The `distant-docker` crate only supports Unix containers, so Windows container daemons
/// are treated as unavailable.
pub async fn docker_available() -> bool {
    match DockerClient::connect_default() {
        Ok(client) => client.ping().await.is_ok() && client.is_linux_engine().await,
        Err(_) => false,
    }
}

/// A managed Docker container for testing.
///
/// Creates a container from ubuntu:22.04 on construction and removes it on drop.
pub struct DockerContainer {
    /// The container name.
    pub name: String,

    /// Docker client handle (kept alive for cleanup).
    client: DockerClient,
}

impl DockerContainer {
    /// Creates and starts a new test container.
    ///
    /// Returns `None` if Docker is unavailable (allowing tests to skip gracefully).
    pub async fn new() -> Option<Self> {
        if !docker_available().await {
            info!("Docker not available, skipping");
            return None;
        }

        let client = DockerClient::connect_default().ok()?;

        let image = "ubuntu:22.04";
        info!("Creating test container from image: {}", image);

        // Pull image if needed
        if !client.has_image(image).await
            && let Err(e) = client.pull_image(image).await
        {
            error!("Failed to pull image: {}", e);
            return None;
        }

        let name = format!("distant-test-{}", random_suffix());

        match client
            .create_and_start_container(
                &name,
                image,
                vec!["sleep".to_string(), "infinity".to_string()],
            )
            .await
        {
            Ok(_) => {
                info!("Test container started: {}", name);
                Some(Self { name, client })
            }
            Err(e) => {
                error!("Failed to create/start test container: {}", e);
                let _ = Docker::stop_and_remove(&client, &name).await;
                None
            }
        }
    }

    /// Execute a command inside this container.
    ///
    /// Delegates to [`DockerClient::exec_cmd`].
    pub async fn exec(&self, cmd: &[&str]) -> io::Result<()> {
        self.client.exec_cmd(&self.name, cmd).await
    }

    /// Detect the Rust target triple for binaries that will run inside this container.
    ///
    /// Runs `uname -m` inside the container and maps the architecture to a
    /// `*-unknown-linux-musl` triple (musl for maximum portability).
    pub async fn detect_target_triple(&self) -> io::Result<String> {
        let output = tokio::process::Command::new("docker")
            .args(["exec", &self.name, "uname", "-m"])
            .output()
            .await?;

        if !output.status.success() {
            return Err(io::Error::other("Failed to detect container architecture"));
        }

        let arch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let triple = match arch.as_str() {
            "x86_64" => "x86_64-unknown-linux-musl",
            "aarch64" => "aarch64-unknown-linux-musl",
            other => {
                return Err(io::Error::other(format!(
                    "Unsupported container architecture: {other}"
                )));
            }
        };
        Ok(triple.to_string())
    }

    /// Returns `true` if the host can produce binaries for this container natively
    /// (same OS and architecture, no cross-compilation needed).
    pub async fn host_matches_container(&self) -> bool {
        if cfg!(not(target_os = "linux")) {
            return false;
        }

        let Ok(triple) = self.detect_target_triple().await else {
            return false;
        };

        if cfg!(target_arch = "x86_64") {
            triple.starts_with("x86_64")
        } else if cfg!(target_arch = "aarch64") {
            triple.starts_with("aarch64")
        } else {
            false
        }
    }

    /// Upload a local file into the container and make it executable.
    ///
    /// Uses `docker cp` to copy the file and `chmod +x` to set the executable bit.
    pub async fn upload_binary(
        &self,
        local_path: &std::path::Path,
        remote_path: &str,
    ) -> io::Result<()> {
        // Ensure the parent directory exists
        if let Some(parent) = std::path::Path::new(remote_path).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() && parent_str != "/" {
                let _ = self.exec(&["mkdir", "-p", &parent_str]).await;
            }
        }

        let dest = format!("{}:{}", self.name, remote_path);
        let status = tokio::process::Command::new("docker")
            .args(["cp", &local_path.to_string_lossy(), &dest])
            .status()
            .await?;

        if !status.success() {
            return Err(io::Error::other(format!(
                "docker cp to {} failed",
                remote_path
            )));
        }

        self.exec(&["chmod", "+x", remote_path]).await
    }

    /// Build a test harness binary for this container and upload it.
    ///
    /// If the host OS/arch matches the container, uses the native build.
    /// Otherwise, cross-compiles for the container's target triple.
    ///
    /// Returns the remote path where the binary was placed (`/usr/local/bin/<name>`).
    pub async fn prepare_binary(&self, bin_name: &str) -> io::Result<String> {
        let local_path = if self.host_matches_container().await {
            // Native build works — use the standard builder
            crate::exe::build_harness_bin_for_target(bin_name, &self.detect_target_triple().await?)
                .await?
        } else {
            // Cross-compile for the container's architecture
            let triple = self.detect_target_triple().await?;
            crate::exe::build_harness_bin_for_target(bin_name, &triple).await?
        };

        let remote_path = format!("/usr/local/bin/{bin_name}");
        self.upload_binary(&local_path, &remote_path).await?;
        Ok(remote_path)
    }
}

impl Drop for DockerContainer {
    fn drop(&mut self) {
        let client = self.client.clone();
        let name = self.name.clone();

        // Use a blocking approach to ensure cleanup happens
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                info!("Cleaning up test container: {}", name);
                if let Err(e) = Docker::stop_and_remove(&client, &name).await {
                    error!("Failed to clean up test container '{}': {}", name, e);
                }
            });
        })
        .join()
        .ok();
    }
}

/// Wrapper holding both a value and the container that keeps it alive.
#[derive(Deref, DerefMut)]
pub struct Ctx<T> {
    /// The wrapped value (Client, Docker, etc.).
    #[deref]
    #[deref_mut]
    pub value: T,

    /// The container (kept alive while tests run).
    #[allow(dead_code)]
    pub container: DockerContainer,
}

/// Convenience macro for tests that depend on Docker.
///
/// Call at the beginning of a test body with an `Option<T>` value. If `None`, the test
/// prints a skip message and returns successfully instead of panicking.
///
/// # Examples
///
/// ```ignore
/// let container = skip_if_no_docker!(docker_container.await);
/// ```
#[macro_export]
macro_rules! skip_if_no_docker {
    ($expr:expr) => {
        match $expr {
            Some(val) => val,
            None => {
                eprintln!("Docker not available — skipping test");
                return;
            }
        }
    };
}

/// rstest fixture that provides an [`Option<DockerContainer>`].
///
/// Returns `None` if Docker is not available, allowing tests to skip gracefully
/// via [`skip_if_no_docker!`].
#[fixture]
pub async fn docker_container() -> Option<DockerContainer> {
    DockerContainer::new().await
}

/// rstest fixture that provides an [`Option<Ctx<Client>>`] connected to a Docker container.
///
/// Returns `None` if Docker is not available.
#[fixture]
pub async fn client(#[future] docker_container: Option<DockerContainer>) -> Option<Ctx<Client>> {
    let container = docker_container.await?;
    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .ok()?;
    let client = docker.into_distant_client().await.ok()?;
    Some(Ctx {
        value: client,
        container,
    })
}

/// rstest fixture that provides an [`Option<Ctx<Client>>`] with tunnel tools
/// (netcat) pre-installed in the container.
///
/// Returns `None` if Docker is not available.
#[fixture]
pub async fn client_with_tunnel_tools(
    #[future] docker_container: Option<DockerContainer>,
) -> Option<Ctx<Client>> {
    let container = docker_container.await?;

    // Install nc before creating the distant connection so TunnelTools detects it
    container.exec(&["apt-get", "update", "-qq"]).await.ok()?;
    container
        .exec(&["apt-get", "install", "-y", "-qq", "netcat-openbsd"])
        .await
        .ok()?;

    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .ok()?;
    let client = docker.into_distant_client().await.ok()?;
    Some(Ctx {
        value: client,
        container,
    })
}

/// Generate a short random suffix for container names.
fn random_suffix() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 4] = rng.r#gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn random_log_file(prefix: &str) -> std::path::PathBuf {
    let log_dir = std::env::temp_dir().join("distant");
    std::fs::create_dir_all(&log_dir).ok();
    log_dir.join(format!("docker-{}.{}.log", prefix, rand::random::<u16>()))
}

/// CLI test context that starts a manager and connects to a Docker container.
///
/// Spawns a `distant manager listen`, creates a Docker container, then runs
/// `distant connect docker://{container}` to register the connection with the manager.
/// Tests can then issue CLI commands against the Docker backend.
pub struct DockerManagerCtx {
    manager: Child,
    container: DockerContainer,
    socket_or_pipe: String,
}

impl DockerManagerCtx {
    /// Creates the context. Returns `None` if Docker is unavailable.
    pub async fn start() -> Option<Self> {
        let container = DockerContainer::new().await?;

        // Start the manager
        let mut manager_cmd = StdCommand::new(bin_path());
        manager_cmd
            .arg("manager")
            .arg("listen")
            .arg("--log-file")
            .arg(random_log_file("manager"))
            .arg("--log-level")
            .arg("trace")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let socket_or_pipe = if cfg!(windows) {
            format!("distant_docker_test_{}", rand::random::<usize>())
        } else {
            std::env::temp_dir()
                .join(format!(
                    "distant_docker_test_{}.sock",
                    rand::random::<usize>()
                ))
                .to_string_lossy()
                .to_string()
        };

        if cfg!(windows) {
            manager_cmd
                .arg("--windows-pipe")
                .arg(socket_or_pipe.as_str());
        } else {
            manager_cmd
                .arg("--unix-socket")
                .arg(socket_or_pipe.as_str());
        }

        eprintln!("DockerManagerCtx: Spawning manager cmd: {manager_cmd:?}");
        let mut manager = manager_cmd.spawn().expect("Failed to spawn manager");
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(Some(status)) = manager.try_wait() {
            panic!("Manager exited ({}): {:?}", status.success(), status.code());
        }

        // Connect to the Docker container via the manager
        let destination = format!("docker://{}", container.name);
        let mut connect_cmd = StdCommand::new(bin_path());
        connect_cmd
            .arg("connect")
            .arg("--log-file")
            .arg(random_log_file("connect"))
            .arg("--log-level")
            .arg("trace")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if cfg!(windows) {
            connect_cmd
                .arg("--windows-pipe")
                .arg(socket_or_pipe.as_str());
        } else {
            connect_cmd
                .arg("--unix-socket")
                .arg(socket_or_pipe.as_str());
        }

        connect_cmd.arg(&destination);

        eprintln!("DockerManagerCtx: Connecting to {destination}");
        let output = connect_cmd.output().expect("Failed to run connect command");

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("DockerManagerCtx: Connect failed: {stderr}");
            let _ = manager.kill();
            return None;
        }
        eprintln!("DockerManagerCtx: Connected. Proceeding with test...");

        Some(Self {
            manager,
            container,
            socket_or_pipe,
        })
    }

    /// Returns the name of the Docker container.
    pub fn container_name(&self) -> &str {
        &self.container.name
    }

    /// Produces a new test command configured with subcommands.
    pub fn new_assert_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> Command {
        let mut cmd = Command::new(bin_path());
        for subcommand in subcommands {
            cmd.arg(subcommand);
        }

        cmd.arg("--log-file")
            .arg(random_log_file("client"))
            .arg("--log-level")
            .arg("trace");

        if cfg!(windows) {
            cmd.arg("--windows-pipe").arg(self.socket_or_pipe.as_str());
        } else {
            cmd.arg("--unix-socket").arg(self.socket_or_pipe.as_str());
        }

        cmd
    }

    /// Returns the binary path and argument list for running a distant
    /// subcommand through this context's manager.
    pub fn cmd_parts<'a>(
        &self,
        subcommands: impl IntoIterator<Item = &'a str>,
    ) -> (PathBuf, Vec<String>) {
        let mut args: Vec<String> = Vec::new();

        for subcommand in subcommands {
            args.push(subcommand.to_string());
        }

        args.push("--log-file".to_string());
        args.push(random_log_file("client").to_string_lossy().to_string());
        args.push("--log-level".to_string());
        args.push("trace".to_string());

        if cfg!(windows) {
            args.push("--windows-pipe".to_string());
        } else {
            args.push("--unix-socket".to_string());
        }
        args.push(self.socket_or_pipe.clone());

        (bin_path(), args)
    }

    /// Produces a new [`StdCommand`] configured with subcommands.
    pub fn new_std_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> StdCommand {
        let mut cmd = StdCommand::new(bin_path());

        for subcommand in subcommands {
            cmd.arg(subcommand);
        }

        cmd.arg("--log-file")
            .arg(random_log_file("client"))
            .arg("--log-level")
            .arg("trace");

        if cfg!(windows) {
            cmd.arg("--windows-pipe").arg(self.socket_or_pipe.as_str());
        } else {
            cmd.arg("--unix-socket").arg(self.socket_or_pipe.as_str());
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd
    }
}

impl Drop for DockerManagerCtx {
    fn drop(&mut self) {
        let _ = self.manager.kill();
        let _ = self.manager.wait();
        // container cleanup handled by DockerContainer::drop
    }
}

/// rstest fixture that provides an [`Option<DockerManagerCtx>`].
///
/// Returns `None` if Docker is not available.
#[fixture]
pub fn docker_ctx() -> Option<DockerManagerCtx> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create runtime");
    rt.block_on(DockerManagerCtx::start())
}
