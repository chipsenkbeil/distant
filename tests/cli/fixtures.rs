use assert_cmd::Command;
use derive_more::{Deref, DerefMut};
use once_cell::sync::Lazy;
use rstest::*;
use serde_json::json;
use std::{
    io,
    path::PathBuf,
    process::{Child, Command as StdCommand, Stdio},
    thread,
    time::Duration,
};

mod repl;
pub use repl::Repl;

static ROOT_LOG_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::temp_dir().join("distant"));
static SESSION_RANDOM: Lazy<u16> = Lazy::new(rand::random);
const TIMEOUT: Duration = Duration::from_secs(3);

// Number of times to retry launching a server before giving up
const LAUNCH_RETRY_CNT: usize = 2;
const LAUNCH_RETRY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Deref, DerefMut)]
pub struct CtxCommand<T> {
    pub ctx: DistantManagerCtx,

    #[deref]
    #[deref_mut]
    pub cmd: T,
}

/// Context for some listening distant server
pub struct DistantManagerCtx {
    manager: Child,
    socket_or_pipe: String,
}

impl DistantManagerCtx {
    /// Starts a manager and server so that clients can connect
    pub fn start() -> Self {
        eprintln!("Logging to {:?}", ROOT_LOG_DIR.as_path());
        std::fs::create_dir_all(ROOT_LOG_DIR.as_path()).expect("Failed to create root log dir");

        // Start the manager
        let mut manager_cmd = StdCommand::new(bin_path());
        manager_cmd
            .arg("manager")
            .arg("listen")
            .arg("--log-file")
            .arg(random_log_file("manager"))
            .arg("--log-level")
            .arg("trace");

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

        eprintln!("Spawning manager cmd: {manager_cmd:?}");
        let mut manager = manager_cmd.spawn().expect("Failed to spawn manager");
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(Some(status)) = manager.try_wait() {
            panic!("Manager exited ({}): {:?}", status.success(), status.code());
        }

        // Spawn a server locally by launching it through the manager
        let mut launch_cmd = StdCommand::new(bin_path());
        launch_cmd
            .arg("client")
            .arg("launch")
            .arg("--log-file")
            .arg(random_log_file("launch"))
            .arg("--log-level")
            .arg("trace")
            .arg("--distant")
            .arg(bin_path())
            .arg("--distant-args")
            .arg(format!(
                "--log-file {} --log-level trace",
                random_log_file("server").to_string_lossy()
            ));

        if cfg!(windows) {
            launch_cmd
                .arg("--windows-pipe")
                .arg(socket_or_pipe.as_str());
        } else {
            launch_cmd.arg("--unix-socket").arg(socket_or_pipe.as_str());
        }

        launch_cmd.arg("manager://localhost");

        for i in 0..=LAUNCH_RETRY_CNT {
            eprintln!("[{i}/{LAUNCH_RETRY_CNT}] Spawning launch cmd: {launch_cmd:?}");
            let output = launch_cmd.output().expect("Failed to launch server");
            let success = output.status.success();
            if success {
                break;
            }

            if !success && i == LAUNCH_RETRY_CNT {
                let _ = manager.kill();
                panic!(
                    "Failed to launch: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            thread::sleep(LAUNCH_RETRY_TIMEOUT);
        }

        Self {
            manager,
            socket_or_pipe,
        }
    }

    pub fn shutdown(&self) -> io::Result<()> {
        // Send a shutdown request to the manager
        let mut shutdown_cmd = StdCommand::new(bin_path());
        shutdown_cmd
            .arg("manager")
            .arg("shutdown")
            .arg("--log-file")
            .arg(random_log_file("shutdown"))
            .arg("--log-level")
            .arg("trace");

        if cfg!(windows) {
            shutdown_cmd
                .arg("--windows-pipe")
                .arg(self.socket_or_pipe.as_str());
        } else {
            shutdown_cmd
                .arg("--unix-socket")
                .arg(self.socket_or_pipe.as_str());
        }

        eprintln!("Spawning shutdown cmd: {shutdown_cmd:?}");
        let output = shutdown_cmd.output().expect("Failed to shutdown server");
        if !output.status.success() {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Failed to shutdown: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            ))
        } else {
            Ok(())
        }
    }

    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
    pub fn new_assert_cmd(&self, subcommands: impl IntoIterator<Item = &'static str>) -> Command {
        let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).expect("Failed to create cmd");
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

    /// Configures some distant command with an environment that can talk to a
    /// remote distant server, spawning it as a child process
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

/// Path to distant binary
fn bin_path() -> PathBuf {
    assert_cmd::cargo::cargo_bin(env!("CARGO_PKG_NAME"))
}

fn random_log_file(prefix: &str) -> PathBuf {
    ROOT_LOG_DIR.join(format!(
        "{}.{}.{}.log",
        prefix,
        *SESSION_RANDOM,
        rand::random::<u16>()
    ))
}

impl Drop for DistantManagerCtx {
    /// Kills manager upon drop
    fn drop(&mut self) {
        // Attempt to shutdown gracefully, forcing a kill otherwise
        if self.shutdown().is_err() {
            let _ = self.manager.kill();
            let _ = self.manager.wait();
        }
    }
}

#[fixture]
pub fn ctx() -> DistantManagerCtx {
    DistantManagerCtx::start()
}

#[fixture]
pub fn lsp_cmd(ctx: DistantManagerCtx) -> CtxCommand<Command> {
    let cmd = ctx.new_assert_cmd(vec!["client", "lsp"]);
    CtxCommand { ctx, cmd }
}

#[fixture]
pub fn action_cmd(ctx: DistantManagerCtx) -> CtxCommand<Command> {
    let cmd = ctx.new_assert_cmd(vec!["client", "action"]);
    CtxCommand { ctx, cmd }
}

#[fixture]
pub fn action_std_cmd(ctx: DistantManagerCtx) -> CtxCommand<StdCommand> {
    let cmd = ctx.new_std_cmd(vec!["client", "action"]);
    CtxCommand { ctx, cmd }
}

#[fixture]
pub fn json_repl(ctx: DistantManagerCtx) -> CtxCommand<Repl> {
    let child = ctx
        .new_std_cmd(vec!["client", "repl"])
        .arg("--format")
        .arg("json")
        .spawn()
        .expect("Failed to start distant repl with json format");
    let cmd = Repl::new(child, TIMEOUT);

    CtxCommand { ctx, cmd }
}

pub async fn validate_authentication(repl: &mut Repl) {
    // NOTE: We have to handle receiving authentication messages, as we will get
    //       an authentication initialization of with method "none", and then
    //       a finish authentication status before we can do anything else.
    let json = repl
        .read_json_from_stdout()
        .await
        .unwrap()
        .expect("Missing authentication initialization");
    assert_eq!(
        json,
        json!({"type": "auth_initialization", "methods": ["none"]})
    );

    let json = repl
        .write_and_read_json(json!({
            "type": "auth_initialization_response",
            "methods": ["none"]
        }))
        .await
        .unwrap()
        .expect("Missing authentication method");
    assert_eq!(json, json!({"type": "auth_start_method", "method": "none"}));

    let json = repl
        .read_json_from_stdout()
        .await
        .unwrap()
        .expect("Missing authentication finalization");
    assert_eq!(json, json!({"type": "auth_finished"}));
}
