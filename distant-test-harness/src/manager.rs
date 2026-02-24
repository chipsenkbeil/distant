use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use derive_more::{Deref, DerefMut};
use distant_core::Credentials;
use distant_core::net::common::Host;
use once_cell::sync::Lazy;
use rstest::*;
use serde_json::{Value, json};
use tokio::sync::mpsc;

static ROOT_LOG_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::temp_dir().join("distant"));
static SESSION_RANDOM: Lazy<u16> = Lazy::new(rand::random);
const TIMEOUT: Duration = Duration::from_secs(3);

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_PAUSE_DURATION: Duration = Duration::from_millis(250);

const CHANNEL_BUFFER: usize = 100;

#[derive(Deref, DerefMut)]
pub struct CtxCommand<T> {
    pub ctx: ManagerCtx,

    #[deref]
    #[deref_mut]
    pub cmd: T,
}

/// Context for some listening distant server
pub struct ManagerCtx {
    manager: Child,
    server: Child,
    socket_or_pipe: String,
}

impl ManagerCtx {
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

        let mut server = None;
        'outer: for i in 1..=MAX_RETRY_ATTEMPTS {
            let mut err = String::new();

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
            server = match server_cmd.spawn() {
                Ok(server) => Some(server),
                Err(x) => {
                    eprintln!("--- SERVER LOG ---");
                    eprintln!(
                        "{}",
                        std::fs::read_to_string(server_log_file.as_path())
                            .unwrap_or_else(|_| format!("Unable to read: {server_log_file:?}"))
                    );
                    eprintln!("------------------");
                    if i == MAX_RETRY_ATTEMPTS {
                        panic!("Failed to spawn server: {x}");
                    } else {
                        continue;
                    }
                }
            };

            // Spawn a thread to read stdout to look for credentials
            let stdout = server.as_mut().unwrap().stdout.take().unwrap();
            let stdout_thread = thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut lines = String::new();
                let mut buf = [0u8; 1024];
                while let Ok(n) = reader.read(&mut buf) {
                    lines.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if let Some(credentials) = Credentials::find(&lines, /* strict */ false) {
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

            let mut credentials = match stdout_thread.join() {
                Ok(credentials) => credentials,
                Err(x) => {
                    if let Err(x) = server.as_mut().unwrap().kill() {
                        eprintln!("Encountered error, but failed to kill server: {x}");
                    }

                    if i == MAX_RETRY_ATTEMPTS {
                        panic!("Failed to retrieve credentials: {x:?}");
                    } else {
                        eprintln!("Failed to retrieve credentials: {x:?}");
                        continue;
                    }
                }
            };

            for host in [
                Host::Ipv4(Ipv4Addr::LOCALHOST),
                Host::Ipv6(Ipv6Addr::LOCALHOST),
                Host::Name("localhost".to_string()),
            ] {
                credentials.host = host.clone();
                // Connect manager to server
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

                connect_cmd.arg(credentials.to_string());

                eprintln!(
                    "[{i}/{MAX_RETRY_ATTEMPTS}] Host: {host} | Spawning connect cmd: {connect_cmd:?}"
                );
                let output = connect_cmd.output().expect("Failed to connect to server");

                if output.status.success() {
                    break 'outer;
                }

                err = format!(
                    "{err}\nConnecting to host {host} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            if let Err(x) = server.as_mut().unwrap().kill() {
                eprintln!("Failed to connect, and failed to kill server: {x}");
            }

            if i == MAX_RETRY_ATTEMPTS {
                eprintln!("--- SERVER LOG ---");
                eprintln!(
                    "{}",
                    std::fs::read_to_string(server_log_file.as_path())
                        .unwrap_or_else(|_| format!("Unable to read: {server_log_file:?}"))
                );
                eprintln!("------------------");

                panic!("Connecting to server failed: {err}");
            } else {
                thread::sleep(RETRY_PAUSE_DURATION);
            }
        }

        eprintln!("Connected! Proceeding with test...");
        Self {
            manager,
            server: server.unwrap(),
            socket_or_pipe,
        }
    }

    /// Produces a new test command configured with a singular subcommand. Useful for root-level
    /// subcommands.
    #[inline]
    pub fn cmd(&self, subcommand: &'static str) -> Command {
        self.new_assert_cmd(vec![subcommand])
    }

    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
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

        eprintln!("new_assert_cmd: {cmd:?}");
        cmd
    }

    /// Returns the binary path and argument list for a distant command.
    /// Useful when an external library (e.g. `expectrl`) needs to spawn the process.
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

        eprintln!("new_std_cmd: {cmd:?}");
        cmd
    }
}

/// Global override for the distant binary path.
static BIN_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Explicitly set the path to the distant binary.
/// Must be called before any test invokes `bin_path()`.
/// Returns `Err` if already set.
pub fn set_bin_path(path: PathBuf) -> Result<(), PathBuf> {
    BIN_PATH.set(path)
}

/// Path to distant binary (cached after first resolution).
pub fn bin_path() -> PathBuf {
    BIN_PATH.get_or_init(resolve_bin_path).clone()
}

fn resolve_bin_path() -> PathBuf {
    let name = if cfg!(windows) {
        "distant.exe"
    } else {
        "distant"
    };

    // 1. Compile-time path from Cargo (works for test targets in the same package)
    if let Some(path) = option_env!("CARGO_BIN_EXE_distant") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }

    // 2. Runtime env var (in case binary was rebuilt after test compilation)
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_distant") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return p;
        }
    }

    // 3. Locate via workspace root derived from CARGO_MANIFEST_DIR.
    //    Available at compile time for every crate (including library crates).
    //    For distant-test-harness: {workspace_root}/distant-test-harness/
    //    Cargo always hard-links final binaries into target/{profile}/ even
    //    when build-dir is configured elsewhere.
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(workspace_root) = manifest_dir.parent() {
            let target_dir = workspace_root.join("target");
            for subdir in [
                "debug",
                "release",
                "llvm-cov-target/debug",
                "llvm-cov-target/release",
            ] {
                let candidate = target_dir.join(subdir).join(name);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    // 4. Check CARGO_TARGET_DIR at runtime (if set, binary lives there instead of target/)
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        for profile in ["debug", "release"] {
            let candidate = PathBuf::from(&target_dir).join(profile).join(name);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // 5. Fall back to PATH
    which::which("distant").expect(
        "distant binary not found: not in CARGO_BIN_EXE_distant, \
         not under workspace target/, not in CARGO_TARGET_DIR, \
         and not on PATH",
    )
}

fn random_log_file(prefix: &str) -> PathBuf {
    ROOT_LOG_DIR.join(format!(
        "{}.{}.{}.log",
        prefix,
        *SESSION_RANDOM,
        rand::random::<u16>()
    ))
}

impl Drop for ManagerCtx {
    /// Kills manager upon drop
    fn drop(&mut self) {
        let _ = self.manager.kill();
        let _ = self.server.kill();
        let _ = self.manager.wait();
        let _ = self.server.wait();
    }
}

/// Context that starts a manager and server but does NOT auto-connect.
/// Exposes server credentials so tests can manually call `distant connect`.
pub struct ManagerOnlyCtx {
    manager: Child,
    server: Child,
    socket_or_pipe: String,
    /// The credentials string for connecting to the server (e.g. "distant://:key@host:port")
    pub credentials: String,
}

impl ManagerOnlyCtx {
    /// Starts a manager and a server, returning credentials without connecting.
    pub fn start() -> Self {
        eprintln!("ManagerOnlyCtx: Logging to {:?}", ROOT_LOG_DIR.as_path());
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

        eprintln!("ManagerOnlyCtx: Spawning manager cmd: {manager_cmd:?}");
        let mut manager = manager_cmd.spawn().expect("Failed to spawn manager");
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(Some(status)) = manager.try_wait() {
            panic!("Manager exited ({}): {:?}", status.success(), status.code());
        }

        // Spawn a server and capture the credentials
        let mut server_cmd = StdCommand::new(bin_path());
        server_cmd
            .arg("server")
            .arg("listen")
            .arg("--log-file")
            .arg(random_log_file("server"))
            .arg("--log-level")
            .arg("trace")
            .arg("--shutdown")
            .arg("lonely=60")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        eprintln!("ManagerOnlyCtx: Spawning server cmd: {server_cmd:?}");
        let mut server = server_cmd.spawn().expect("Failed to spawn server");

        // Read stdout to extract credentials
        let stdout = server.stdout.take().unwrap();
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut lines = String::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                lines.push_str(&String::from_utf8_lossy(&buf[..n]));
                if let Some(credentials) = Credentials::find(&lines, /* strict */ false) {
                    return credentials;
                }
            }
            panic!("Failed to read credentials from server stdout");
        });

        // Wait for thread to finish (up to 500ms)
        let start = Instant::now();
        while !stdout_thread.is_finished() {
            if start.elapsed() > Duration::from_millis(500) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        let credentials = stdout_thread
            .join()
            .expect("Failed to retrieve credentials from server");

        eprintln!("ManagerOnlyCtx: Got credentials, NOT connecting. Proceeding...");
        Self {
            manager,
            server,
            socket_or_pipe,
            credentials: credentials.to_string(),
        }
    }

    /// Produces a new test command configured with subcommands and connected to this manager.
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

        eprintln!("ManagerOnlyCtx::new_assert_cmd: {cmd:?}");
        cmd
    }

    /// Produces a new std::process::Command configured with subcommands.
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

        eprintln!("ManagerOnlyCtx::new_std_cmd: {cmd:?}");
        cmd
    }

    /// Returns the socket/pipe path used by this manager.
    pub fn socket_or_pipe(&self) -> &str {
        &self.socket_or_pipe
    }
}

impl Drop for ManagerOnlyCtx {
    fn drop(&mut self) {
        let _ = self.manager.kill();
        let _ = self.server.kill();
        let _ = self.manager.wait();
        let _ = self.server.wait();
    }
}

#[fixture]
pub fn manager_only_ctx() -> ManagerOnlyCtx {
    ManagerOnlyCtx::start()
}

#[fixture]
pub fn ctx() -> ManagerCtx {
    ManagerCtx::start()
}

#[fixture]
pub fn lsp_cmd(ctx: ManagerCtx) -> CtxCommand<Command> {
    let cmd = ctx.new_assert_cmd(vec!["lsp"]);
    CtxCommand { ctx, cmd }
}

#[fixture]
pub fn action_std_cmd(ctx: ManagerCtx) -> CtxCommand<StdCommand> {
    let cmd = ctx.new_std_cmd(vec!["action"]);
    CtxCommand { ctx, cmd }
}

#[fixture]
pub fn api_process(ctx: ManagerCtx) -> CtxCommand<ApiProcess> {
    let child = ctx
        .new_std_cmd(vec!["api"])
        .spawn()
        .expect("Failed to start distant api with json format");
    let cmd = ApiProcess::new(child, TIMEOUT);

    CtxCommand { ctx, cmd }
}

pub async fn validate_authentication(proc: &mut ApiProcess) {
    // NOTE: We have to handle receiving authentication messages, as we will get
    //       an authentication initialization of with method "none", and then
    //       a finish authentication status before we can do anything else.
    let json = proc
        .read_json_from_stdout()
        .await
        .unwrap()
        .expect("Missing authentication initialization");
    assert_eq!(
        json,
        json!({"type": "auth_initialization", "methods": ["none"]})
    );

    let json = proc
        .write_and_read_json(json!({
            "type": "auth_initialization_response",
            "methods": ["none"]
        }))
        .await
        .unwrap()
        .expect("Missing authentication method");
    assert_eq!(json, json!({"type": "auth_start_method", "method": "none"}));

    let json = proc
        .read_json_from_stdout()
        .await
        .unwrap()
        .expect("Missing authentication finalization");
    assert_eq!(json, json!({"type": "auth_finished"}));
}

// --- ApiProcess ---

pub struct ApiProcess {
    child: Child,
    stdin: mpsc::Sender<String>,
    stdout: mpsc::Receiver<String>,
    stderr: mpsc::Receiver<String>,
    timeout: Option<Duration>,
}

impl ApiProcess {
    /// Create a new [`ApiProcess`] wrapping around a [`Child`]
    pub fn new(mut child: Child, timeout: impl Into<Option<Duration>>) -> Self {
        let mut stdin = BufWriter::new(child.stdin.take().expect("Child missing stdin"));
        let mut stdout = BufReader::new(child.stdout.take().expect("Child missing stdout"));
        let mut stderr = BufReader::new(child.stderr.take().expect("Child missing stderr"));

        let (stdin_tx, mut rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            while let Some(data) = rx.blocking_recv() {
                if stdin.write_all(data.as_bytes()).is_err() {
                    break;
                }

                // NOTE: If we don't do this, the data doesn't appear to get sent even
                //       with a newline at the end. At least in testing thus far!
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        let (tx, stdout_rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            let mut line = String::new();
            while let Ok(n) = stdout.read_line(&mut line) {
                if n == 0 {
                    break;
                }

                if tx.blocking_send(line).is_err() {
                    break;
                }

                line = String::new();
            }
        });

        let (tx, stderr_rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            let mut line = String::new();
            while let Ok(n) = stderr.read_line(&mut line) {
                if n == 0 {
                    break;
                }

                if tx.blocking_send(line).is_err() {
                    break;
                }

                line = String::new();
            }
        });

        Self {
            child,
            stdin: stdin_tx,
            stdout: stdout_rx,
            stderr: stderr_rx,
            timeout: timeout.into(),
        }
    }

    /// Writes json to the api over stdin and then waits for json to be received over stdout,
    /// failing if either operation exceeds timeout if set or if the output to stdout is not json,
    /// and returns none if stdout channel has closed
    pub async fn write_and_read_json(
        &mut self,
        value: impl Into<Value>,
    ) -> io::Result<Option<Value>> {
        self.write_json_to_stdin(value).await?;
        self.read_json_from_stdout().await
    }

    /// Writes a line of input to stdin, failing if exceeds timeout if set or if the stdin channel
    /// has been closed. Will append a newline character (`\n`) if line does not end with one.
    pub async fn write_line_to_stdin(&mut self, line: impl Into<String>) -> io::Result<()> {
        let mut line = line.into();
        if !line.ends_with('\n') {
            line.push('\n');
        }

        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stdin.send(line)).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(x)) => Err(io::Error::new(io::ErrorKind::BrokenPipe, x)),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    self.collect_stderr(),
                )),
            },
            None => self
                .stdin
                .send(line)
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x)),
        }
    }

    /// Writes json value as a line of input to stdin, failing if exceeds timeout if set or if the
    /// stdin channel has been closed. Will append a newline character (`\n`) to JSON string.
    pub async fn write_json_to_stdin(&mut self, value: impl Into<Value>) -> io::Result<()> {
        self.write_line_to_stdin(value.into().to_string()).await
    }

    /// Tries to read a line from stdout, returning none if no stdout is available right now
    ///
    /// Will fail if no more stdout is available
    pub fn try_read_line_from_stdout(&mut self) -> io::Result<Option<String>> {
        match self.stdout.try_recv() {
            Ok(line) => Ok(Some(line)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            }
        }
    }

    /// Reads a line from stdout, failing if exceeds timeout if set, returning none if the stdout
    /// channel has been closed
    pub async fn read_line_from_stdout(&mut self) -> io::Result<Option<String>> {
        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stdout.recv()).await {
                Ok(x) => Ok(x),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    self.collect_stderr(),
                )),
            },
            None => Ok(self.stdout.recv().await),
        }
    }

    /// Reads a line from stdout and parses it as json, failing if unable to parse as json or the
    /// timeout is reached if set, returning none if the stdout channel has been closed
    pub async fn read_json_from_stdout(&mut self) -> io::Result<Option<Value>> {
        match self.read_line_from_stdout().await? {
            Some(line) => {
                let value = serde_json::from_str(&line)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Reads a line from stderr, failing if exceeds timeout if set, returning none if the stderr
    /// channel has been closed
    #[allow(dead_code)]
    pub async fn read_line_from_stderr(&mut self) -> io::Result<Option<String>> {
        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stderr.recv()).await {
                Ok(x) => Ok(x),
                Err(x) => Err(io::Error::new(io::ErrorKind::TimedOut, x)),
            },
            None => Ok(self.stderr.recv().await),
        }
    }

    /// Tries to read a line from stderr, returning none if no stderr is available right now
    ///
    /// Will fail if no more stderr is available
    pub fn try_read_line_from_stderr(&mut self) -> io::Result<Option<String>> {
        match self.stderr.try_recv() {
            Ok(line) => Ok(Some(line)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            }
        }
    }

    /// Collects stderr into a singular string (failures will stop the collection)
    pub fn collect_stderr(&mut self) -> String {
        let mut stderr = String::new();

        while let Ok(Some(line)) = self.try_read_line_from_stderr() {
            stderr.push_str(&line);
        }

        stderr
    }

    /// Kills the api by sending a signal to the process
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

impl Drop for ApiProcess {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
