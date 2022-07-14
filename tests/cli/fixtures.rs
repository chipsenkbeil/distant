use assert_cmd::Command;
use once_cell::sync::Lazy;
use once_cell::sync::OnceCell;
use rstest::*;
use std::{
    path::PathBuf,
    process::{Child, Command as StdCommand, Stdio},
    time::Duration,
};

mod repl;
pub use repl::Repl;

static LOG_PATH: Lazy<PathBuf> =
    Lazy::new(|| std::env::temp_dir().join("test.distant.manager.log"));
const TIMEOUT: Duration = Duration::from_secs(3);

/// Context for some listening distant server
pub struct DistantManagerCtx {
    manager: Child,
    socket_or_pipe: String,
}

impl DistantManagerCtx {
    /// Starts a manager and server so that clients can connect
    pub fn start() -> Self {
        // Start the manager
        let mut manager_cmd = StdCommand::new(bin_path());
        manager_cmd
            .arg("manager")
            .arg("listen")
            .arg("--log-file")
            .arg(LOG_PATH.as_path());

        let socket_or_pipe = if cfg!(windows) {
            format!("distant_test_{}", rand::random::<usize>())
        } else {
            std::env::temp_dir()
                .join(format!("distant_test_{}.sock", rand::random::<usize>()))
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

        let mut manager = manager_cmd.spawn().expect("Failed to spawn manager");
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(Some(status)) = manager.try_wait() {
            panic!("Manager exited ({}): {:?}", status.success(), status.code());
        }

        // Spawn a server locally by launching it through the manager
        let mut launch_cmd = StdCommand::new(bin_path());
        launch_cmd.arg("client").arg("launch");

        if cfg!(windows) {
            launch_cmd
                .arg("--windows-pipe")
                .arg(socket_or_pipe.as_str());
        } else {
            launch_cmd.arg("--unix-socket").arg(socket_or_pipe.as_str());
        }

        let output = launch_cmd
            .arg("manager://localhost")
            .output()
            .expect("Failed to launch server");
        if !output.status.success() {
            let _ = manager.kill();
            panic!(
                "Failed to launch: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Self {
            manager,
            socket_or_pipe,
        }
    }

    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
    pub fn new_assert_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> Command {
        let mut command = Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to create cmd");
        for cmd in subcommands {
            command.arg(cmd);
        }

        if cfg!(windows) {
            command
                .arg("--windows-pipe")
                .arg(self.socket_or_pipe.as_str());
        } else {
            command
                .arg("--unix-socket")
                .arg(self.socket_or_pipe.as_str());
        }

        command
    }

    /// Configures some distant command with an environment that can talk to a
    /// remote distant server, spawning it as a child process
    pub fn new_std_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> StdCommand {
        let mut cmd = StdCommand::new(bin_path());

        for subcommand in subcommands {
            cmd.arg(subcommand);
        }

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

/// Path to distant binary
fn bin_path() -> PathBuf {
    assert_cmd::cargo::cargo_bin(env!("CARGO_PKG_NAME"))
}

impl Drop for DistantManagerCtx {
    /// Kills manager upon drop
    ///
    /// NOTE: This is never triggered
    fn drop(&mut self) {
        let _ = self.manager.kill();
    }
}

#[fixture]
pub fn ctx() -> &'static DistantManagerCtx {
    static CTX: OnceCell<DistantManagerCtx> = OnceCell::new();

    CTX.get_or_init(DistantManagerCtx::start)
}

#[fixture]
pub fn lsp_cmd(ctx: &'_ DistantManagerCtx) -> Command {
    ctx.new_assert_cmd(vec!["client", "lsp"])
}

#[fixture]
pub fn action_cmd(ctx: &'_ DistantManagerCtx) -> Command {
    ctx.new_assert_cmd(vec!["client", "action"])
}

#[fixture]
pub fn action_std_cmd(ctx: &'_ DistantManagerCtx) -> StdCommand {
    ctx.new_std_cmd(vec!["client", "action"])
}

#[fixture]
pub fn json_repl(ctx: &'_ DistantManagerCtx) -> Repl {
    let child = ctx
        .new_std_cmd(vec!["client", "repl"])
        .arg("--format")
        .arg("json")
        .spawn()
        .expect("Failed to start distant repl with json format");
    Repl::new(child, TIMEOUT)
}
