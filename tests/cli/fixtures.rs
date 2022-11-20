use assert_cmd::Command;
use derive_more::{Deref, DerefMut};
use distant_core::{net::common::Host, DistantSingleKeyCredentials};
use once_cell::sync::Lazy;
use rstest::*;
use serde_json::json;
use std::{
    io::{BufReader, Read},
    net::{Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    process::{Child, Command as StdCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

mod repl;
pub use repl::Repl;

static ROOT_LOG_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::temp_dir().join("distant"));
static SESSION_RANDOM: Lazy<u16> = Lazy::new(rand::random);
const TIMEOUT: Duration = Duration::from_secs(3);

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_PAUSE_DURATION: Duration = Duration::from_millis(250);

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
    server: Child,
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
            .arg("trace")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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

        // Spawn a server and capture the credentials so we can connect to it
        let mut server_cmd = StdCommand::new(bin_path());
        let server_log_file = random_log_file("server");
        server_cmd
            .arg("server")
            .arg("listen")
            .arg("--log-file")
            .arg(&server_log_file)
            .arg("--log-level")
            .arg("trace")
            .arg("--shutdown")
            .arg("lonely=60")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        eprintln!("Spawning server cmd: {server_cmd:?}");
        let mut server = server_cmd.spawn().expect("Failed to spawn server");

        // Spawn a thread to read stdout to look for credentials
        let stdout = server.stdout.take().unwrap();
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut lines = String::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                lines.push_str(&String::from_utf8_lossy(&buf[..n]));
                if let Some(credentials) = DistantSingleKeyCredentials::find(&lines) {
                    return credentials;
                }
            }
            panic!("Failed to read line");
        });

        // Wait for thread to finish (up to 500ms)
        let start = Instant::now();
        while !stdout_thread.is_finished() {
            if start.elapsed() > Duration::from_millis(500) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        let mut credentials = stdout_thread.join().unwrap();

        'outer: for i in 1..=MAX_RETRY_ATTEMPTS {
            let mut err = String::new();

            for host in vec![
                Host::Ipv4(Ipv4Addr::LOCALHOST),
                Host::Ipv6(Ipv6Addr::LOCALHOST),
                Host::Name("localhost".to_string()),
            ] {
                credentials.host = host.clone();
                // Connect manager to server
                let mut connect_cmd = StdCommand::new(bin_path());
                connect_cmd
                    .arg("client")
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

                connect_cmd.arg(credentials.to_string());

                eprintln!("[{i}/{MAX_RETRY_ATTEMPTS}] Host: {host} | Spawning connect cmd: {connect_cmd:?}");
                let output = connect_cmd.output().expect("Failed to connect to server");

                if output.status.success() {
                    break 'outer;
                }

                err = String::from_utf8_lossy(&output.stderr).to_string();
            }

            if i == MAX_RETRY_ATTEMPTS {
                eprintln!("--- SERVER LOG ---");
                eprintln!(
                    "{}",
                    std::fs::read_to_string(server_log_file.as_path())
                        .unwrap_or_else(|_| format!("Unable to read: {server_log_file:?}"))
                );
                eprintln!("------------------");

                panic!("Connecting to server failed: {}", err);
            } else {
                thread::sleep(RETRY_PAUSE_DURATION);
            }
        }

        eprintln!("Connected! Proceeding with test...");
        Self {
            manager,
            server,
            socket_or_pipe,
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
        let _ = self.manager.kill();
        let _ = self.server.kill();
        let _ = self.manager.wait();
        let _ = self.server.wait();
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
