use crate::{subcommand, data::Operation};
use derive_more::{Display, Error, From};
use lazy_static::lazy_static;
use std::{
    env,
    net::{AddrParseError, IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
};
use structopt::StructOpt;

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

    /// Quiet mode
    #[structopt(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, StructOpt)]
pub enum Subcommand {
    #[structopt(visible_aliases = &["exec", "x"])]
    Execute(ExecuteSubcommand),
    Launch(LaunchSubcommand),
    Listen(ListenSubcommand),
}

impl Subcommand {
    /// Runs the subcommand, returning the result
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Execute(cmd) => subcommand::execute::run(cmd)?,
            Self::Launch(cmd) => subcommand::launch::run(cmd)?,
            Self::Listen(cmd) => subcommand::listen::run(cmd)?,
        }

        Ok(())
    }
}

#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
pub enum ExecuteFormat {
    #[display(fmt = "shell")]
    Shell,
    #[display(fmt = "json")]
    Json,
}

#[derive(Clone, Debug, Display, From, Error, PartialEq, Eq)]
pub enum ExecuteFormatParseError {
    InvalidVariant(#[error(not(source))] String),
}

impl FromStr for ExecuteFormat {
    type Err = ExecuteFormatParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "shell" => Ok(Self::Shell),
            "json" => Ok(Self::Json),
            x => Err(ExecuteFormatParseError::InvalidVariant(x.to_string())),
        }
    }
}

/// Represents subcommand to execute some operation remotely
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
pub struct ExecuteSubcommand {
    /// Represents the format that results should be returned
    ///
    /// Currently, there are two possible formats:
    /// 1. "shell": printing out human-readable results for interactive shell usage
    /// 2. "json": printing our JSON for external program usage
    #[structopt(
        short, 
        long, 
        value_name = "shell|json", 
        default_value = "shell", 
        possible_values = &["shell", "json"]
    )]
    pub format: ExecuteFormat,

    #[structopt(subcommand)]
    pub operation: Operation,
}

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
pub enum BindAddress {
    #[display(fmt = "ssh")]
    Ssh,
    #[display(fmt = "any")]
    Any,
    Ip(IpAddr),
}

#[derive(Clone, Debug, Display, From, Error, PartialEq, Eq)]
pub enum ConvertToIpAddrError {
    ClientIpParseError(AddrParseError),
    MissingClientIp,
    VarError(env::VarError),
}

impl BindAddress {
    /// Converts address into valid IP
    pub fn to_ip_addr(&self) -> Result<IpAddr, ConvertToIpAddrError> {
        match self {
            Self::Ssh => {
                let ssh_connection = env::var("SSH_CONNECTION")?;
                let ip_str = ssh_connection
                    .split(' ')
                    .next()
                    .ok_or(ConvertToIpAddrError::MissingClientIp)?;
                let ip = ip_str.parse::<IpAddr>()?;
                Ok(ip)
            }
            Self::Any => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Self::Ip(addr) => Ok(*addr),
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
            x => x.parse(),
        }
    }
}

/// Represents subcommand to launch a remote server
#[derive(Debug, StructOpt)]
pub struct LaunchSubcommand {
    /// Outputs port and key of remotely-started binary
    #[structopt(long)]
    pub print_startup_data: bool,

    /// Path to remote program to execute via ssh
    #[structopt(short, long, default_value = "distant")]
    pub remote_program: String,

    /// Path to ssh program to execute
    #[structopt(short, long, default_value = "ssh")]
    pub ssh_program: String,

    /// Control the IP address that the mosh-server binds to.
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

    /// Username to use when sshing into remote machine
    #[structopt(short, long, default_value = &USERNAME)]
    pub username: String,

    /// Explicit identity file to use with ssh
    #[structopt(short, long)]
    pub identity_file: Option<PathBuf>,

    /// Port to use for sshing into the remote machine
    #[structopt(short, long, default_value = "22")]
    pub port: u16,

    /// Host to use for sshing into the remote machine
    #[structopt(name = "ADDRESS")]
    pub host: String,
}

/// Represents some range of ports
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRange {
    pub start: u16,
    pub end: Option<u16>,
}

impl PortRange {
    /// Builds a collection of `SocketAddr` instances from the port range and given ip address
    pub fn make_socket_addrs(&self, addr: impl Into<IpAddr>) -> Vec<SocketAddr> {
        let mut socket_addrs = Vec::new();
        let addr = addr.into();

        for port in self.start..=self.end.unwrap_or(self.start) {
            socket_addrs.push(SocketAddr::from((addr, port)));
        }

        socket_addrs
    }
}

#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum PortRangeParseError {
    InvalidPort,
    MissingPort,
}

impl FromStr for PortRange {
    type Err = PortRangeParseError;

    /// Parses PORT into single range or PORT1:PORTN into full range
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.trim().split(':');
        let start = tokens
            .next()
            .ok_or(PortRangeParseError::MissingPort)?
            .parse::<u16>()
            .map_err(|_| PortRangeParseError::InvalidPort)?;
        let end = if let Some(token) = tokens.next() {
            Some(
                token
                    .parse::<u16>()
                    .map_err(|_| PortRangeParseError::InvalidPort)?,
            )
        } else {
            None
        };

        if tokens.next().is_some() {
            return Err(PortRangeParseError::InvalidPort);
        }

        Ok(Self { start, end })
    }
}

/// Represents subcommand to operate in listen mode for incoming requests
#[derive(Debug, StructOpt)]
pub struct ListenSubcommand {
    /// Runs in background via daemon-mode (does nothing on windows)
    #[structopt(short, long)]
    pub daemon: bool,

    /// Prevents output of selected port, key, and other info
    #[structopt(long)]
    pub no_print_startup_data: bool,

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

    /// Set the port(s) that the server will attempt to bind to
    ///
    /// This can be in the form of PORT1 or PORT1:PORTN to provide a range of ports.
    /// With -p 0, the server will let the operating system pick an available TCP port.
    ///
    /// Please note that this option does not affect the server-side port used by SSH
    #[structopt(
        short,
        long,
        value_name = "PORT[:PORT2]",
        default_value = "60000:61000"
    )]
    pub port: PortRange,
}
