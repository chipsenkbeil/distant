use lazy_static::lazy_static;
use std::{net::SocketAddr, path::PathBuf};
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
    #[structopt(visible_aliases = &["conn", "c"])]
    Connect(ConnectSubcommand),
    #[structopt(visible_aliases = &["exec", "x"])]
    Execute(ExecuteSubcommand),
    Launch(LaunchSubcommand),
    #[structopt(visible_aliases = &["l"])]
    Listen(ListenSubcommand),
}

/// Represents subcommand to connect to an already-running remote server
#[derive(Debug, StructOpt)]
pub struct ConnectSubcommand {}

/// Represents subcommand to execute some operation remotely
#[derive(Debug, StructOpt)]
pub struct ExecuteSubcommand {}

/// Represents subcommand to launch a remote server
#[derive(Debug, StructOpt)]
pub struct LaunchSubcommand {
    /// Username to use when sshing into remote machine
    #[structopt(short, long, default_value = &USERNAME)]
    pub username: String,

    /// Explicit identity file to use with ssh
    #[structopt(short, long)]
    pub identity_file: Option<PathBuf>,

    /// Destination of remote machine to launch binary
    #[structopt(name = "DESTINATION")]
    pub destination: SocketAddr,
}

/// Represents subcommand to operate in listen mode for incoming requests
#[derive(Debug, StructOpt)]
pub struct ListenSubcommand {}
