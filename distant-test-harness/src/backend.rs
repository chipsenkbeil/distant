//! Cross-plugin backend abstraction for parameterized integration tests.
//!
//! Provides [`BackendCtx`] to unify [`HostManagerCtx`](crate::manager::HostManagerCtx),
//! [`SshManagerCtx`](crate::manager::SshManagerCtx), and
//! [`DockerManagerCtx`](crate::docker::DockerManagerCtx) behind a single
//! interface. Tests can use rstest `#[case]` parameters to run the same
//! assertion against multiple backends.

use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};

use assert_cmd::Command;

use crate::manager;

/// Identifies which plugin backend a test should exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The local host backend (no network hop).
    Host,

    /// The SSH plugin backend (connects through a per-test sshd).
    Ssh,

    /// The Docker plugin backend (connects to an ephemeral container).
    Docker,
}

/// A test context wrapping one of the supported plugin backends.
///
/// Each variant holds the corresponding context type, which manages the
/// manager process, connection, and cleanup. The enum delegates the common
/// command-building methods so tests can remain backend-agnostic.
pub enum BackendCtx {
    /// Host backend context.
    Host(manager::HostManagerCtx),

    /// SSH backend context.
    Ssh(manager::SshManagerCtx),

    /// Docker backend context (only available with the `docker` feature).
    #[cfg(feature = "docker")]
    Docker(crate::docker::DockerManagerCtx),
}

impl BackendCtx {
    /// Returns which backend this context uses.
    pub fn backend(&self) -> Backend {
        match self {
            Self::Host(_) => Backend::Host,
            Self::Ssh(_) => Backend::Ssh,
            #[cfg(feature = "docker")]
            Self::Docker(_) => Backend::Docker,
        }
    }

    /// Produces a new [`assert_cmd::Command`] configured with the given subcommands.
    ///
    /// The returned command is pre-configured to communicate with the
    /// manager that this context owns (socket/pipe, logging, etc.).
    pub fn new_assert_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> Command {
        match self {
            Self::Host(ctx) => ctx.new_assert_cmd(subcommands),
            Self::Ssh(ctx) => ctx.new_assert_cmd(subcommands),
            #[cfg(feature = "docker")]
            Self::Docker(ctx) => ctx.new_assert_cmd(subcommands),
        }
    }

    /// Produces a new [`std::process::Command`] configured with the given subcommands.
    ///
    /// The returned command is pre-configured with piped stdio and the
    /// manager socket/pipe for this context.
    pub fn new_std_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> StdCommand {
        match self {
            Self::Host(ctx) => ctx.new_std_cmd(subcommands),
            Self::Ssh(ctx) => ctx.new_std_cmd(subcommands),
            #[cfg(feature = "docker")]
            Self::Docker(ctx) => ctx.new_std_cmd(subcommands),
        }
    }

    /// Returns the binary path and argument list for running a distant
    /// subcommand through this context's manager.
    ///
    /// Useful when spawning commands through non-standard mechanisms
    /// (e.g., `portable-pty` for PTY tests) that need raw `(PathBuf, Vec<String>)`.
    pub fn cmd_parts<'a>(
        &self,
        subcommands: impl IntoIterator<Item = &'a str>,
    ) -> (PathBuf, Vec<String>) {
        match self {
            Self::Host(ctx) => ctx.cmd_parts(subcommands),
            Self::Ssh(ctx) => ctx.cmd_parts(subcommands),
            #[cfg(feature = "docker")]
            Self::Docker(ctx) => ctx.cmd_parts(subcommands),
        }
    }

    /// Produces a new [`assert_cmd::Command`] configured with a single subcommand.
    #[inline]
    pub fn cmd(&self, subcommand: &'static str) -> Command {
        self.new_assert_cmd(vec![subcommand])
    }

    /// Returns a unique temp directory path valid for the backend's filesystem.
    ///
    /// Docker containers always use `/tmp/` (Linux). Host and SSH use the
    /// platform's temp directory (also `/tmp/` on Unix, `%TEMP%` on Windows).
    pub fn unique_dir(&self, label: &str) -> String {
        let id: u64 = rand::random();
        let base = match self.backend() {
            Backend::Docker => PathBuf::from("/tmp"),
            _ => std::env::temp_dir(),
        };
        base.join(format!("distant-test-{label}-{id}"))
            .to_string_lossy()
            .to_string()
    }

    /// Joins a child filename to a parent directory, using the correct
    /// path separator for the backend.
    ///
    /// Docker always uses `/` (Linux). Host and SSH use the platform separator.
    pub fn child_path(&self, dir: &str, name: &str) -> String {
        match self.backend() {
            Backend::Docker => format!("{dir}/{name}"),
            _ => PathBuf::from(dir).join(name).to_string_lossy().to_string(),
        }
    }

    /// Creates a file through the distant CLI, working across all backends.
    ///
    /// # Panics
    ///
    /// Panics if the write command fails.
    pub fn cli_write(&self, path: &str, content: &str) {
        let mut child = self
            .new_std_cmd(["fs", "write"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn fs write");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "fs write setup failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Reads a file through the distant CLI, working across all backends.
    ///
    /// # Panics
    ///
    /// Panics if the read command fails.
    pub fn cli_read(&self, path: &str) -> String {
        let output = self
            .new_std_cmd(["fs", "read"])
            .arg(path)
            .output()
            .expect("failed to run fs read");
        assert!(
            output.status.success(),
            "fs read failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// Checks if a path exists through the distant CLI, working across all backends.
    pub fn cli_exists(&self, path: &str) -> bool {
        let output = self
            .new_std_cmd(["fs", "exists"])
            .arg(path)
            .output()
            .expect("failed to run fs exists");
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("true")
    }

    /// Returns the Docker container name if this is a Docker backend.
    #[cfg(feature = "docker")]
    pub fn docker_container_name(&self) -> Option<&str> {
        match self {
            Self::Docker(ctx) => Some(ctx.container_name()),
            _ => None,
        }
    }

    /// Builds a test harness binary and returns a path usable by the backend.
    ///
    /// For Host and SSH backends, the binary is built natively and the local
    /// path is returned. For Docker, the binary is cross-compiled, uploaded
    /// to the container, and the remote path is returned.
    pub async fn prepare_binary(&self, bin_name: &str) -> io::Result<String> {
        match self {
            #[cfg(feature = "docker")]
            Self::Docker(ctx) => ctx.container().prepare_binary(bin_name).await,
            _ => {
                let path = crate::exe::build_harness_bin(bin_name, None).await?;
                Ok(path.to_string_lossy().to_string())
            }
        }
    }

    /// Creates a directory through the distant CLI, working across all backends.
    ///
    /// # Panics
    ///
    /// Panics if the make-dir command fails.
    pub fn cli_mkdir(&self, path: &str) {
        let output = self
            .new_std_cmd(["fs", "make-dir"])
            .arg(path)
            .output()
            .expect("failed to run fs make-dir");
        assert!(
            output.status.success(),
            "fs make-dir failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Creates a symbolic link through the distant CLI, working across all
    /// backends and platforms.
    ///
    /// `target` is the path the symlink points to; `link` is the path of the
    /// symlink itself. For SSH and Docker backends the remote is always Unix
    /// so `ln -s` is used. For the Host backend, the command is
    /// platform-dependent: `ln -s` on Unix, `cmd /c mklink` on Windows.
    ///
    /// # Panics
    ///
    /// Panics if the symlink creation command fails.
    pub fn cli_symlink(&self, target: &str, link: &str) {
        let output = if cfg!(windows) && !matches!(self.backend(), Backend::Docker) {
            // Detect whether the target is a directory so we can pass `/D` to mklink.
            let is_dir = {
                let meta = self
                    .new_std_cmd(["fs", "metadata"])
                    .arg(target)
                    .output()
                    .expect("failed to check target type");
                meta.status.success() && String::from_utf8_lossy(&meta.stdout).contains("Type: dir")
            };
            let mut args = vec!["--", "cmd", "/c", "mklink"];
            if is_dir {
                args.push("/D");
            }
            args.extend([link, target]);
            self.new_std_cmd(["spawn"])
                .args(&args)
                .output()
                .expect("failed to create symlink")
        } else {
            self.new_std_cmd(["spawn"])
                .args(["--", "ln", "-s", target, link])
                .output()
                .expect("failed to create symlink")
        };
        assert!(
            output.status.success(),
            "symlink creation failed ({target} -> {link}): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Creates a [`BackendCtx`] for the given backend.
///
/// Returns `None` when the backend's prerequisites are not available:
/// - [`Backend::Host`] is always available.
/// - [`Backend::Ssh`] requires `sshd` to be installed on the system.
/// - [`Backend::Docker`] requires the `docker` feature and a reachable
///   Linux Docker daemon.
pub fn ctx_for_backend(backend: Backend) -> Option<BackendCtx> {
    match backend {
        Backend::Host => Some(BackendCtx::Host(manager::HostManagerCtx::start())),
        Backend::Ssh => {
            which::which("sshd").ok()?;
            Some(BackendCtx::Ssh(manager::SshManagerCtx::start()))
        }
        Backend::Docker => ctx_for_docker(),
    }
}

/// Attempts to create a Docker backend context.
///
/// Separated so the `#[cfg]` gate is in one place rather than scattered
/// across match arms.
#[cfg(feature = "docker")]
fn ctx_for_docker() -> Option<BackendCtx> {
    // Spawn the Docker context creation on a separate thread to avoid
    // "Cannot start a runtime from within a runtime" when called from
    // within a #[tokio::test] context.
    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(crate::docker::DockerManagerCtx::start())
            .map(BackendCtx::Docker)
    })
    .join()
    .expect("Docker context thread panicked")
}

/// Returns `None` when the `docker` feature is not enabled.
#[cfg(not(feature = "docker"))]
fn ctx_for_docker() -> Option<BackendCtx> {
    None
}

/// Creates a [`BackendCtx`] for the given [`Backend`], skipping the test if
/// the backend's prerequisites are unavailable.
///
/// Calls [`ctx_for_backend`] internally — callers only need to pass the
/// [`Backend`] variant.
///
/// # Examples
///
/// ```ignore
/// let ctx = skip_if_no_backend!(Backend::Docker);
/// ```
#[macro_export]
macro_rules! skip_if_no_backend {
    ($backend:expr) => {
        match $crate::backend::ctx_for_backend($backend) {
            Some(val) => val,
            None => {
                eprintln!("Backend not available — skipping test");
                return;
            }
        }
    };
}
