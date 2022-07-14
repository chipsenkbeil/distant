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

static LOG_PATH: Lazy<PathBuf> = Lazy::new(|| std::env::temp_dir().join("test.distant.server.log"));
const TIMEOUT: Duration = Duration::from_secs(15);

/// Context for some listening distant server
pub struct DistantManagerCtx {
    manager: Child,
}

impl DistantManagerCtx {
    /// Starts a manager and server so that clients can connect
    pub fn start() -> Self {
        // Start the manager
        let mut manager = StdCommand::new(bin_path())
            .arg("manager")
            .arg("listen")
            .arg("--log-file")
            .arg(LOG_PATH.as_path())
            .spawn()
            .expect("Failed to spawn manager");

        // Spawn a server locally by launching it through the manager
        let output = StdCommand::new(bin_path())
            .arg("client")
            .arg("launch")
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

        Self { manager }
    }

    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
    pub fn new_assert_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> Command {
        let mut command = Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to create cmd");
        for cmd in subcommands {
            command.arg(cmd);
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
