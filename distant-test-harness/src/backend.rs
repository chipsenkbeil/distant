//! Cross-plugin backend abstraction for parameterized integration tests.
//!
//! Provides [`BackendCtx`] to unify [`ManagerCtx`](crate::manager::ManagerCtx),
//! [`SshManagerCtx`](crate::manager::SshManagerCtx), and
//! [`DockerManagerCtx`](crate::docker::DockerManagerCtx) behind a single
//! interface. Tests can use rstest `#[case]` parameters to run the same
//! assertion against multiple backends.

use std::process::Command as StdCommand;

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
    Host(manager::ManagerCtx),

    /// SSH backend context.
    Ssh(manager::SshManagerCtx),

    /// Docker backend context (only available with the `docker` feature).
    #[cfg(feature = "docker")]
    Docker(crate::docker::DockerManagerCtx),
}

impl BackendCtx {
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
        Backend::Host => Some(BackendCtx::Host(manager::ManagerCtx::start())),
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

/// Unwraps an `Option`, skipping the test if the value is `None`.
///
/// Call at the beginning of a test body with a value returned by
/// [`ctx_for_backend`]. If the backend is unavailable, the test prints a
/// skip message and returns successfully instead of panicking.
///
/// # Examples
///
/// ```ignore
/// let ctx = skip_if_no_backend!(ctx_for_backend(Backend::Docker));
/// ```
#[macro_export]
macro_rules! skip_if_no_backend {
    ($expr:expr) => {
        match $expr {
            Some(val) => val,
            None => {
                eprintln!("Backend not available — skipping test");
                return;
            }
        }
    };
}
