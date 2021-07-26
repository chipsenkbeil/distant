use crate::subcommand;
use derive_more::Display;
use lazy_static::lazy_static;
use std::{
    net::{AddrParseError, IpAddr, Ipv4Addr},
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

/// Represents subcommand to execute some operation remotely
#[derive(Debug, StructOpt)]
pub struct ExecuteSubcommand {}

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
pub enum BindAddress {
    #[display(fmt = "ssh")]
    Ssh,
    #[display(fmt = "any")]
    Any,
    Ip(IpAddr),
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
    pub print_startup_info: bool,

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
    #[structopt(long, default_value = "ssh")]
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

/// Represents subcommand to operate in listen mode for incoming requests
#[derive(Debug, StructOpt)]
pub struct ListenSubcommand {
    /// Runs in background via daemon-mode (does nothing on windows)
    #[structopt(short, long)]
    pub daemon: bool,

    /// Prevents output of selected port, key, and other info
    #[structopt(long)]
    pub no_print_startup_info: bool,

    /// If specified, will attempt to bind to SSH_CONNECTION instead of host
    #[structopt(long)]
    pub bind_ssh_connection: bool,

    /// Control the IP address that the distant binds to. There are three options here:
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
    #[structopt(short, long, default_value = "localhost")]
    pub host: String,

    /// Represents the port to bind to when listening
    #[structopt(short, long, default_value = "60000")]
    pub port: u16,

    /// Represents total range of ports to try if a port is already taken
    /// when binding, applying range incrementally against the specified
    /// port (e.g. 60000-61000 inclusively if range is 1000)
    #[structopt(long, default_value = "1000")]
    pub port_range: u16,
}
