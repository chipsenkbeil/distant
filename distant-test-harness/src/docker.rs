//! Docker container test harness for integration tests.
//!
//! Provides [`DockerContainer`] for managing test container lifecycles and rstest fixtures
//! for obtaining [`Client`] instances connected to Docker containers.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};

use tokio::process::Command as TokioCommand;

use assert_cmd::Command;
use derive_more::{Deref, DerefMut};
use distant_core::Client;
use distant_docker::{Docker, DockerClient, DockerOpts};
use log::*;
use rstest::*;

use crate::manager::{self, bin_path};
use crate::process;

/// Docker image used for building test binaries inside containers.
const DOCKER_BUILD_IMAGE: &str = "rust:1.88-slim";

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

    /// Upload a local file into the container and make it executable.
    ///
    /// Uses `docker cp` to copy the file and `chmod +x` to set the executable bit.
    pub async fn upload_binary(&self, local_path: &Path, remote_path: &str) -> io::Result<()> {
        // Ensure the parent directory exists
        if let Some(parent) = Path::new(remote_path).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() && parent_str != "/" {
                let _ = self.exec(&["mkdir", "-p", &parent_str]).await;
            }
        }

        let dest = format!("{}:{}", self.name, remote_path);
        let status = TokioCommand::new("docker")
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

    /// Detect the container's architecture via `uname -m`.
    async fn container_arch(&self) -> io::Result<String> {
        let output = TokioCommand::new("docker")
            .args(["exec", &self.name, "uname", "-m"])
            .output()
            .await?;

        if !output.status.success() {
            return Err(io::Error::other("Failed to detect container architecture"));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Maps a `uname -m` value to a Rust target triple for cross-compilation.
    fn target_triple_for_arch(arch: &str) -> io::Result<&'static str> {
        match arch {
            "x86_64" => Ok("x86_64-unknown-linux-gnu"),
            "aarch64" => Ok("aarch64-unknown-linux-gnu"),
            other => Err(io::Error::other(format!(
                "Unsupported container architecture: {other}"
            ))),
        }
    }

    /// Build a test harness binary for this container and upload it.
    ///
    /// Tries cross-compilation with `--target` first (fast if a cross-linker
    /// is installed or the host already matches). Falls back to building inside
    /// a [`DOCKER_BUILD_IMAGE`] Docker container with a minimal generated project.
    ///
    /// Returns the remote path where the binary was placed (`/usr/local/bin/<name>`).
    pub async fn prepare_binary(&self, bin_name: &str) -> io::Result<String> {
        let arch = self.container_arch().await?;
        let triple = Self::target_triple_for_arch(&arch)?;

        // Fast path: try cross-compile (works natively on matching Linux hosts,
        // or when a cross-linker like `aarch64-linux-gnu-gcc` is installed)
        let local_path = match crate::exe::build_harness_bin(bin_name, Some(triple)).await {
            Ok(path) => {
                log::info!("Cross-compiled {bin_name} for {triple}");
                path
            }
            Err(e) => {
                log::info!("Cross-compile failed ({e}), falling back to Docker build");
                build_in_docker(bin_name).await?
            }
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

/// Returns the minimal Cargo.toml content for a standalone binary.
///
/// Each test binary only needs a fraction of the dependencies that the full
/// `distant-test-harness` crate pulls in. Building a minimal project inside
/// Docker is dramatically faster than compiling the entire workspace.
///
/// Returns a `String` so that templates can be built dynamically.
fn minimal_cargo_toml(bin_name: &str) -> String {
    let deps = match bin_name {
        "pty-echo" => "",
        "pty-password" => "rpassword = \"7\"\n",
        "pty-interactive" => "ctrlc = \"3\"\nrpassword = \"7\"\n",
        "tcp-echo-server" => {
            "tokio = { version = \"1\", features = [\"net\", \"io-util\", \"time\", \"macros\", \"rt-multi-thread\"] }\n"
        }
        "tcp-to-stdio" => {
            "tokio = { version = \"1\", features = [\"net\", \"io-util\", \"io-std\", \"macros\", \"rt-multi-thread\"] }\n"
        }
        _ => panic!("unknown test binary: {bin_name}"),
    };

    let mut toml = format!(
        "\
[package]
name = \"{bin_name}\"
version = \"0.0.0\"
edition = \"2024\"
"
    );

    if !deps.is_empty() {
        toml.push_str("\n[dependencies]\n");
        toml.push_str(deps);
    }

    toml.push_str(&format!(
        "\n[[bin]]\nname = \"{bin_name}\"\npath = \"main.rs\"\n"
    ));

    toml
}

/// Returns the source file path for a test binary (underscore-named in src/bin/).
fn bin_source_path(bin_name: &str) -> PathBuf {
    let file_name = format!("{}.rs", bin_name.replace('-', "_"));
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("bin")
        .join(file_name)
}

/// Builds a test binary inside Docker using a minimal generated Cargo project.
///
/// Instead of compiling the entire distant workspace, this creates a tiny project
/// with only the dependencies the binary actually needs (e.g., just `tokio` for
/// tcp-echo-server). The result is cached on disk so subsequent runs are instant.
async fn build_in_docker(bin_name: &str) -> io::Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");

    let cache_dir = workspace_root.join("target").join("docker-build");
    std::fs::create_dir_all(&cache_dir)?;

    let output_path = cache_dir.join(bin_name);

    if output_path.exists() {
        log::info!(
            "Using cached Docker-built binary: {}",
            output_path.display()
        );
        return Ok(output_path);
    }

    log::info!("Building {bin_name} inside Docker (minimal project)...");

    // Create a temp directory with a minimal Cargo project
    let build_dir = cache_dir.join(format!("{bin_name}-src"));
    std::fs::create_dir_all(&build_dir)?;
    std::fs::write(build_dir.join("Cargo.toml"), minimal_cargo_toml(bin_name))?;
    std::fs::copy(bin_source_path(bin_name), build_dir.join("main.rs"))?;

    let build_dir_str = build_dir.to_string_lossy();
    let cache_dir_str = cache_dir.to_string_lossy();
    let target_dir = "/tmp/build";

    // Copy source to a writable location inside the container (Cargo needs to
    // write Cargo.lock), then build and copy the result to the output volume.
    let build_and_copy = format!(
        "cp -r /src /build && cd /build \
         && cargo build --release --target-dir {target_dir} \
         && cp {target_dir}/release/{bin_name} /out/{bin_name}"
    );

    let status = TokioCommand::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{build_dir_str}:/src:ro"),
            "-v",
            &format!("{cache_dir_str}:/out"),
            "-v",
            "distant-docker-cargo-cache:/usr/local/cargo/registry",
            "-w",
            "/src",
            DOCKER_BUILD_IMAGE,
            "sh",
            "-c",
            &build_and_copy,
        ])
        .status()
        .await?;

    let _ = std::fs::remove_dir_all(&build_dir);

    if !status.success() {
        return Err(io::Error::other(format!(
            "Docker build of {bin_name} failed"
        )));
    }

    if !output_path.exists() {
        return Err(io::Error::other(format!(
            "Docker build completed but {} not found",
            output_path.display()
        )));
    }

    Ok(output_path)
}

/// Generate a short random suffix for container names.
fn random_suffix() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 4] = rng.r#gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn random_log_file(prefix: &str) -> PathBuf {
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

        process::set_process_group(&mut manager_cmd);
        eprintln!("DockerManagerCtx: Spawning manager cmd: {manager_cmd:?}");
        let mut manager = manager_cmd.spawn().expect("Failed to spawn manager");
        manager::wait_for_manager_ready(&socket_or_pipe, &mut manager);

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

    /// Returns a reference to the underlying [`DockerContainer`].
    pub fn container(&self) -> &DockerContainer {
        &self.container
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
        process::kill_process_tree(&mut self.manager);
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
