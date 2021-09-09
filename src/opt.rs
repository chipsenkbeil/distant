use crate::{
    constants::{
        SERVER_CONN_MSG_CAPACITY_STR, SESSION_FILE_PATH_STR, SESSION_SOCKET_PATH_STR, TIMEOUT_STR,
    },
    exit::ExitCodeError,
    subcommand,
};
use derive_more::{Display, Error, From, IsVariant};
use distant_core::{PortRange, RequestData};
use lazy_static::lazy_static;
use std::{
    env,
    net::{AddrParseError, IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};
use structopt::StructOpt;
use strum::{EnumString, EnumVariantNames, IntoStaticStr, VariantNames};

lazy_static! {
    static ref USERNAME: String = whoami::username();
}

/// Options and commands to apply to binary
#[derive(Debug, StructOpt)]
#[structopt(name = "distant")]
pub struct Opt {
    #[structopt(flatten)]
    pub common: CommonOpt,

    #[structopt(subcommand)]
    pub subcommand: Subcommand,
}

impl Opt {
    /// Loads options from CLI arguments
    pub fn load() -> Self {
        Self::from_args()
    }
}

/// Contains options that are common across subcommands
#[derive(Debug, StructOpt)]
pub struct CommonOpt {
    /// Verbose mode (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences), global = true)]
    pub verbose: u8,

    /// Quiet mode, suppresses all logging
    #[structopt(short, long, global = true)]
    pub quiet: bool,

    /// Log output to disk instead of stderr
    #[structopt(long, global = true)]
    pub log_file: Option<PathBuf>,

    /// Represents the maximum time (in seconds) to wait for a network
    /// request before timing out; a timeout of 0 implies waiting indefinitely
    #[structopt(short, long, global = true, default_value = &TIMEOUT_STR)]
    pub timeout: f32,
}

impl CommonOpt {
    /// Creates a new duration representing the timeout in seconds
    pub fn to_timeout_duration(&self) -> Duration {
        Duration::from_secs_f32(self.timeout)
    }
}

/// Contains options related sessions
#[derive(Debug, StructOpt)]
pub struct SessionOpt {
    /// Represents the location of the file containing session information,
    /// only useful when the session is set to "file"
    #[structopt(long, default_value = &SESSION_FILE_PATH_STR)]
    pub session_file: PathBuf,

    /// Represents the location of the session's socket to communicate across,
    /// only useful when the session is set to "socket"
    #[structopt(long, default_value = &SESSION_SOCKET_PATH_STR)]
    pub session_socket: PathBuf,
}

#[derive(Debug, StructOpt)]
pub enum Subcommand {
    /// Performs some action on a remote machine
    Action(ActionSubcommand),

    /// Launches the server-portion of the binary on a remote machine
    Launch(LaunchSubcommand),

    /// Begins listening for incoming requests
    Listen(ListenSubcommand),

    /// Specialized treatment of running a remote LSP process
    Lsp(LspSubcommand),
}

impl Subcommand {
    /// Runs the subcommand, returning the result
    pub fn run(self, opt: CommonOpt) -> Result<(), Box<dyn ExitCodeError>> {
        match self {
            Self::Action(cmd) => subcommand::action::run(cmd, opt)?,
            Self::Launch(cmd) => subcommand::launch::run(cmd, opt)?,
            Self::Listen(cmd) => subcommand::listen::run(cmd, opt)?,
            Self::Lsp(cmd) => subcommand::lsp::run(cmd, opt)?,
        }

        Ok(())
    }

    /// Returns true if subcommand simplifies to acting as a proxy for a remote process
    pub fn is_remote_process(&self) -> bool {
        match self {
            Self::Action(cmd) => cmd
                .operation
                .as_ref()
                .map(|req| req.is_proc_run())
                .unwrap_or_default(),
            Self::Lsp(_) => true,
            _ => false,
        }
    }
}

/// Represents the format for data communicated to & from the client
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    PartialEq,
    Eq,
    IsVariant,
    IntoStaticStr,
    EnumString,
    EnumVariantNames,
)]
#[strum(serialize_all = "snake_case")]
pub enum Format {
    /// Sends and receives data in JSON format
    Json,

    /// Commands are traditional shell commands and output responses are
    /// inline with what is expected of a program's output in a shell
    Shell,
}

/// Represents subcommand to execute some operation remotely
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
pub struct ActionSubcommand {
    /// Represents the format that results should be returned
    ///
    /// Currently, there are two possible formats:
    ///
    /// 1. "json": printing out JSON for external program usage
    ///
    /// 2. "shell": printing out human-readable results for interactive shell usage
    #[structopt(
        short,
        long,
        case_insensitive = true,
        default_value = Format::Shell.into(),
        possible_values = Format::VARIANTS
    )]
    pub format: Format,

    /// Represents the medium for retrieving a session for use in performing the action
    #[structopt(
        long,
        default_value = SessionInput::default().into(),
        possible_values = SessionInput::VARIANTS
    )]
    pub session: SessionInput,

    /// Contains additional information related to sessions
    #[structopt(flatten)]
    pub session_data: SessionOpt,

    /// If specified, commands to send are sent over stdin and responses are received
    /// over stdout (and stderr if mode is shell)
    #[structopt(short, long)]
    pub interactive: bool,

    /// Operation to send over the wire if not in interactive mode
    #[structopt(subcommand)]
    pub operation: Option<RequestData>,
}

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, IsVariant)]
pub enum BindAddress {
    #[display(fmt = "ssh")]
    Ssh,
    #[display(fmt = "any")]
    Any,
    Ip(IpAddr),
}

#[derive(Clone, Debug, Display, From, Error, PartialEq, Eq)]
pub enum ConvertToIpAddrError {
    AddrParseError(AddrParseError),
    #[display(fmt = "SSH_CONNECTION missing 3rd argument (host ip)")]
    MissingSshAddr,
    VarError(env::VarError),
}

impl BindAddress {
    /// Converts address into valid IP; in the case of "any", will leverage the
    /// `use_ipv6` flag to determine if binding should use ipv4 or ipv6
    pub fn to_ip_addr(self, use_ipv6: bool) -> Result<IpAddr, ConvertToIpAddrError> {
        match self {
            Self::Ssh => {
                let ssh_connection = env::var("SSH_CONNECTION")?;
                let ip_str = ssh_connection
                    .split(' ')
                    .nth(2)
                    .ok_or(ConvertToIpAddrError::MissingSshAddr)?;
                let ip = ip_str.parse::<IpAddr>()?;
                Ok(ip)
            }
            Self::Any if use_ipv6 => Ok(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            Self::Any => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Self::Ip(addr) => Ok(addr),
        }
    }
}

impl FromStr for BindAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "ssh" => Ok(Self::Ssh),
            "any" => Ok(Self::Any),
            "localhost" => Ok(Self::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            x => Ok(Self::Ip(x.parse::<IpAddr>()?)),
        }
    }
}

/// Represents the means by which to share the session from launching on a remote machine
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    PartialEq,
    Eq,
    IntoStaticStr,
    IsVariant,
    EnumString,
    EnumVariantNames,
)]
#[strum(serialize_all = "snake_case")]
pub enum SessionOutput {
    /// Session is in a file in the form of `DISTANT DATA <host> <port> <auth key>`
    File,

    /// Special scenario where the session is not shared but is instead kept within the
    /// launch program, causing the program itself to listen on stdin for input rather
    /// than terminating
    Keep,

    /// Session is stored and retrieved over anonymous pipes (stdout/stdin)
    /// in form of `DISTANT DATA <host> <port> <auth key>`
    Pipe,

    /// Special scenario where the session is not shared but is instead kept within the
    /// launch program, where the program now listens on a unix socket for input
    #[cfg(unix)]
    Socket,
}

impl Default for SessionOutput {
    /// For unix-based systems, output defaults to a socket
    #[cfg(unix)]
    fn default() -> Self {
        Self::Socket
    }

    /// For non-unix-based systems, output defaults to a file
    #[cfg(not(unix))]
    fn default() -> Self {
        Self::File
    }
}

/// Represents the means by which to consume a session when performing an action
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    PartialEq,
    Eq,
    IntoStaticStr,
    IsVariant,
    EnumString,
    EnumVariantNames,
)]
#[strum(serialize_all = "snake_case")]
pub enum SessionInput {
    /// Session is in a environment variables
    ///
    /// * `DISTANT_HOST=<host>`
    /// * `DISTANT_PORT=<port>`
    /// * `DISTANT_AUTH_KEY=<auth key>`
    Environment,

    /// Session is in a file in the form of `DISTANT DATA <host> <port> <auth key>`
    File,

    /// Session is stored and retrieved over anonymous pipes (stdout/stdin)
    /// in form of `DISTANT DATA <host> <port> <auth key>`
    Pipe,

    /// Session is stored and retrieved from the initializeOptions of the initialize request
    /// that is first sent for an LSP through
    Lsp,

    /// Session isn't directly available but instead there is a process listening
    /// on a unix socket that will forward requests and responses
    #[cfg(unix)]
    Socket,
}

impl Default for SessionInput {
    /// For unix-based systems, input defaults to a socket
    #[cfg(unix)]
    fn default() -> Self {
        Self::Socket
    }

    /// For non-unix-based systems, input defaults to a file
    #[cfg(not(unix))]
    fn default() -> Self {
        Self::File
    }
}

/// Represents subcommand to launch a remote server
#[derive(Debug, StructOpt)]
pub struct LaunchSubcommand {
    /// Represents the medium for sharing the session upon launching on a remote machine
    #[structopt(
        long,
        default_value = SessionOutput::default().into(),
        possible_values = SessionOutput::VARIANTS
    )]
    pub session: SessionOutput,

    /// Contains additional information related to sessions
    #[structopt(flatten)]
    pub session_data: SessionOpt,

    /// If specified, launch will fail when attempting to bind to a unix socket that
    /// already exists, rather than removing the old socket
    #[structopt(long)]
    pub fail_if_socket_exists: bool,

    /// The time in seconds before shutting down the server if there are no active
    /// connections. The countdown begins once all connections have closed and
    /// stops when a new connection is made. In not specified, the server will not
    /// shutdown at any point when there are no active connections.
    ///
    /// In the case of launch, this is only applicable when it is set to socket session
    /// as this controls when the unix socket listener would shutdown, not when the
    /// remote server it is connected to will shutdown.
    ///
    /// To configure the remote server's shutdown time, provide it as an argument
    /// via `--extra-server-args`
    #[structopt(long)]
    pub shutdown_after: Option<f32>,

    /// Runs in background via daemon-mode (does nothing on windows); only applies
    /// when session is socket
    #[structopt(short, long)]
    pub daemon: bool,

    /// Represents the format that results should be returned when session is "keep",
    /// causing the launcher to enter an interactive loop to handle input and output
    /// itself rather than enabling other clients to connect
    #[structopt(
        short,
        long,
        case_insensitive = true,
        default_value = Format::Shell.into(),
        possible_values = Format::VARIANTS
    )]
    pub format: Format,

    /// Path to distant program on remote machine to execute via ssh;
    /// by default, this program needs to be available within PATH as
    /// specified when compiling ssh (not your login shell)
    #[structopt(long, default_value = "distant")]
    pub distant: String,

    /// Path to ssh program on local machine to execute
    #[structopt(long, default_value = "ssh")]
    pub ssh: String,

    /// Control the IP address that the server binds to.
    ///
    /// The default is `ssh', in which case the server will reply from the IP address that the SSH
    /// connection came from (as found in the SSH_CONNECTION environment variable). This is
    /// useful for multihomed servers.
    ///
    /// With --bind-server=any, the server will reply on the default interface and will not bind to
    /// a particular IP address. This can be useful if the connection is made through sslh or
    /// another tool that makes the SSH connection appear to come from localhost.
    ///
    /// With --bind-server=IP, the server will attempt to bind to the specified IP address.
    #[structopt(long, value_name = "ssh|any|IP", default_value = "ssh")]
    pub bind_server: BindAddress,

    /// Additional arguments to provide to the server
    #[structopt(long, allow_hyphen_values(true))]
    pub extra_server_args: Option<String>,

    /// Username to use when sshing into remote machine
    #[structopt(short, long, default_value = &USERNAME)]
    pub username: String,

    /// Explicit identity file to use with ssh
    #[structopt(short, long)]
    pub identity_file: Option<PathBuf>,

    /// If specified, will not launch distant using a login shell but instead execute it directly
    #[structopt(long)]
    pub no_shell: bool,

    /// Port to use for sshing into the remote machine
    #[structopt(short, long, default_value = "22")]
    pub port: u16,

    /// Host to use for sshing into the remote machine
    #[structopt(name = "HOST")]
    pub host: String,
}

impl LaunchSubcommand {
    /// Creates a new duration representing the shutdown period in seconds
    pub fn to_shutdown_after_duration(&self) -> Option<Duration> {
        self.shutdown_after
            .as_ref()
            .copied()
            .map(Duration::from_secs_f32)
    }
}

/// Represents subcommand to operate in listen mode for incoming requests
#[derive(Debug, StructOpt)]
pub struct ListenSubcommand {
    /// Runs in background via daemon-mode (does nothing on windows)
    #[structopt(short, long)]
    pub daemon: bool,

    /// Control the IP address that the distant binds to
    ///
    /// There are three options here:
    ///
    /// 1. `ssh`: the server will reply from the IP address that the SSH
    /// connection came from (as found in the SSH_CONNECTION environment variable). This is
    /// useful for multihomed servers.
    ///
    /// 2. `any`: the server will reply on the default interface and will not bind to
    /// a particular IP address. This can be useful if the connection is made through sslh or
    /// another tool that makes the SSH connection appear to come from localhost.
    ///
    /// 3. `IP`: the server will attempt to bind to the specified IP address.
    #[structopt(short, long, value_name = "ssh|any|IP", default_value = "localhost")]
    pub host: BindAddress,

    /// If specified, will bind to the ipv6 interface if host is "any" instead of ipv4
    #[structopt(short = "6", long)]
    pub use_ipv6: bool,

    /// Maximum capacity for concurrent message handled by the server
    #[structopt(long, default_value = &SERVER_CONN_MSG_CAPACITY_STR)]
    pub max_msg_capacity: u16,

    /// The time in seconds before shutting down the server if there are no active
    /// connections. The countdown begins once all connections have closed and
    /// stops when a new connection is made. In not specified, the server will not
    /// shutdown at any point when there are no active connections.
    #[structopt(long)]
    pub shutdown_after: Option<f32>,

    /// Changes the current working directory (cwd) to the specified directory
    #[structopt(long)]
    pub current_dir: Option<PathBuf>,

    /// Set the port(s) that the server will attempt to bind to
    ///
    /// This can be in the form of PORT1 or PORT1:PORTN to provide a range of ports.
    /// With -p 0, the server will let the operating system pick an available TCP port.
    ///
    /// Please note that this option does not affect the server-side port used by SSH
    #[structopt(short, long, value_name = "PORT[:PORT2]", default_value = "8080:8099")]
    pub port: PortRange,
}

impl ListenSubcommand {
    /// Creates a new duration representing the shutdown period in seconds
    pub fn to_shutdown_after_duration(&self) -> Option<Duration> {
        self.shutdown_after
            .as_ref()
            .copied()
            .map(Duration::from_secs_f32)
    }
}

/// Represents subcommand to execute some LSP server on a remote machine
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
pub struct LspSubcommand {
    /// Represents the format that results should be returned
    ///
    /// Currently, there are two possible formats:
    ///
    /// 1. "json": printing out JSON for external program usage
    ///
    /// 2. "shell": printing out human-readable results for interactive shell usage
    #[structopt(
        short,
        long,
        case_insensitive = true,
        default_value = Format::Shell.into(),
        possible_values = Format::VARIANTS
    )]
    pub format: Format,

    /// Represents the medium for retrieving a session to use when running a remote LSP server
    #[structopt(
        long,
        default_value = SessionInput::default().into(),
        possible_values = SessionInput::VARIANTS
    )]
    pub session: SessionInput,

    /// Contains additional information related to sessions
    #[structopt(flatten)]
    pub session_data: SessionOpt,

    /// Command to run on the remote machine that represents an LSP server
    pub cmd: String,

    /// Additional arguments to supply to the remote machine
    pub args: Vec<String>,
}
