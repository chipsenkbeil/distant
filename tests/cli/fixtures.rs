use assert_cmd::Command;
use once_cell::sync::OnceCell;
use rstest::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    process::{Child, Command as StdCommand, Stdio},
};

const LOG_PATH: PathBuf = std::env::temp_dir().join("test.distant.server.log");

/// Context for some listening distant server
pub struct DistantServerCtx {
    manager: Child,
}

impl DistantServerCtx {
    /// Starts a manager and server so that clients can connect
    pub fn start() -> Self {
        // Start the manager
        let manager = StdCommand::new(bin_path())
            .arg("manager")
            .arg("listen")
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
    pub fn new_assert_cmd(&self, subcommand: impl AsRef<OsStr>) -> Command {
        Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to create cmd")
    }

    /// Configures some distant command with an environment that can talk to a
    /// remote distant server, spawning it as a child process
    pub fn new_std_cmd(&self, subcommand: impl AsRef<OsStr>) -> StdCommand {
        let mut cmd = StdCommand::new(bin_path());

        cmd.arg(subcommand)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd
    }
}

/// Path to distant binary
fn bin_path() -> PathBuf {
    assert_cmd::cargo::cargo_bin(env!("CARGO_PKG_NAME"))
}

impl Drop for DistantServerCtx {
    /// Kills manager upon drop
    fn drop(&mut self) {
        let _ = self.manager.kill();
    }
}

#[fixture]
pub fn ctx() -> &'static DistantServerCtx {
    static CTX: OnceCell<DistantServerCtx> = OnceCell::new();

    CTX.get_or_init(DistantServerCtx::start)
}

#[fixture]
pub fn lsp_cmd(ctx: &'_ DistantServerCtx) -> Command {
    ctx.new_assert_cmd("lsp")
}

#[fixture]
pub fn action_cmd(ctx: &'_ DistantServerCtx) -> Command {
    ctx.new_assert_cmd("action")
}

#[fixture]
pub fn action_std_cmd(ctx: &'_ DistantServerCtx) -> StdCommand {
    ctx.new_std_cmd("action")
}

#[fixture]
pub fn repl_cmd(ctx: &'_ DistantServerCtx) -> Command {
    ctx.new_assert_cmd("repl")
}

#[fixture]
pub fn repl_std_cmd(ctx: &'_ DistantServerCtx) -> StdCommand {
    ctx.new_std_cmd("repl")
}
