use super::common::{AccessControl, LoggingSettings, NetworkSettings, Value};
use crate::constants::user::CACHE_FILE_PATH_STR;
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::Shell as ClapCompleteShell;
use distant_core::data::{DistantRequestData, Environment};
use distant_core::net::common::{ConnectionId, Destination, Map};
use service_manager::ServiceManagerKind;
use std::path::PathBuf;

/// Primary entrypoint into options & subcommands for the CLI.
#[derive(Debug, Parser)]
#[clap(author, version, about)]
#[clap(name = "distant")]
pub struct Options {
    #[clap(flatten)]
    pub logging: LoggingSettings,

    /// Configuration file to load instead of the default paths
    #[clap(short = 'c', long = "config", global = true, value_parser)]
    pub config_path: Option<PathBuf>,

    #[clap(subcommand)]
    pub command: DistantSubcommand,
}

/// Subcommands for the CLI.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum DistantSubcommand {
    /// Perform client commands
    #[clap(subcommand)]
    Client(ClientSubcommand),

    /// Perform manager commands
    #[clap(subcommand)]
    Manager(ManagerSubcommand),

    /// Perform server commands
    #[clap(subcommand)]
    Server(ServerSubcommand),

    /// Perform generation commands
    #[clap(subcommand)]
    Generate(GenerateSubcommand),
}

/// Subcommands for `distant client`.
#[derive(Debug, Subcommand)]
pub enum ClientSubcommand {
    /// Performs some action on a remote machine
    Action {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Represents the maximum time (in seconds) to wait for a network request before timing out.
        #[clap(long)]
        timeout: Option<f32>,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,

        #[clap(subcommand)]
        request: DistantRequestData,
    },

    /// Requests that active manager connects to the server at the specified destination
    Connect {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Additional options to provide, typically forwarded to the handler within the manager
        /// facilitating the connection. Options are key-value pairs separated by comma.
        ///
        /// E.g. `key="value",key2="value2"`
        #[clap(long)]
        options: Option<Map>,

        #[clap(flatten)]
        network: NetworkSettings,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        destination: Box<Destination>,
    },

    /// Launches the server-portion of the binary on a remote machine
    Launch {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Additional options to provide, typically forwarded to the handler within the manager
        /// facilitating the launch of a distant server. Options are key-value pairs separated by
        /// comma.
        ///
        /// E.g. `key="value",key2="value2"`
        #[clap(long)]
        options: Option<Map>,

        #[clap(flatten)]
        network: NetworkSettings,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        destination: Box<Destination>,
    },

    /// Specialized treatment of running a remote LSP process
    Lsp {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// If provided, will run LSP in a pty
        #[clap(long)]
        pty: bool,

        cmd: String,
    },

    /// Runs actions in a read-eval-print loop
    Repl {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        #[clap(flatten)]
        config: ClientReplConfig,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,

        /// Format used for input into and output from the repl
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,
    },

    /// Select the active connection
    Select {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Connection to use, otherwise will prompt to select
        connection: Option<ConnectionId>,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// Specialized treatment of running a remote shell process
    Shell {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// Environment variables to provide to the shell
        #[clap(long, default_value_t)]
        environment: Environment,

        /// Optional command to run instead of $SHELL
        cmd: Option<String>,
    },
}

/// Subcommands for `distant generate`.
#[derive(Debug, Subcommand)]
pub enum GenerateSubcommand {
    /// Generate configuration file with base settings
    Config {
        /// Path to where the configuration file should be created
        file: PathBuf,
    },

    /// Generate JSON schema for server request/response
    Schema {
        /// If specified, will output to the file at the given path instead of stdout
        #[clap(long)]
        file: Option<PathBuf>,
    },

    // Generate completion info for CLI
    Completion {
        /// If specified, will output to the file at the given path instead of stdout
        #[clap(long)]
        file: Option<PathBuf>,

        /// Specific shell to target for the generated output
        #[clap(value_enum, value_parser)]
        shell: ClapCompleteShell,
    },
}

/// Subcommands for `distant manager`.
#[derive(Debug, Subcommand)]
pub enum ManagerSubcommand {
    /// Interact with a manager being run by a service management platform
    #[clap(subcommand)]
    Service(ManagerServiceSubcommand),

    /// Listen for incoming requests as a manager
    Listen {
        /// Type of access to apply to created unix socket or windows pipe
        #[clap(long, value_enum)]
        access: Option<AccessControl>,

        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

        /// If specified, will listen on a user-local unix socket or local windows named pipe
        #[clap(long)]
        user: bool,

        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// Retrieve a list of capabilities that the manager supports
    Capabilities {
        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// Retrieve information about a specific connection
    Info {
        id: ConnectionId,
        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// List information about all connections
    List {
        #[clap(flatten)]
        network: NetworkSettings,

        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,
    },

    /// Kill a specific connection
    Kill {
        #[clap(flatten)]
        network: NetworkSettings,
        id: ConnectionId,
    },
}

/// Subcommands for `distant manager service`.
#[derive(Debug, Subcommand)]
pub enum ManagerServiceSubcommand {
    /// Start the manager as a service
    Start {
        /// Type of service manager used to run this service, defaulting to platform native
        #[clap(long, value_enum)]
        kind: Option<ServiceManagerKind>,

        /// If specified, starts as a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Stop the manager as a service
    Stop {
        #[clap(long, value_enum)]
        kind: Option<ServiceManagerKind>,

        /// If specified, stops a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Install the manager as a service
    Install {
        #[clap(long, value_enum)]
        kind: Option<ServiceManagerKind>,

        /// If specified, installs as a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Uninstall the manager as a service
    Uninstall {
        #[clap(long, value_enum)]
        kind: Option<ServiceManagerKind>,

        /// If specified, uninstalls a user-level service
        #[clap(long)]
        user: bool,
    },
}

/// Subcommands for `distant server`.
#[derive(Debug, Subcommand)]
pub enum ServerSubcommand {
    /// Listen for incoming requests as a server
    Listen {
        #[clap(flatten)]
        config: ServerListenConfig,

        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

        /// If specified, the server will not generate a key but instead listen on stdin for the next
        /// 32 bytes that it will use as the key instead. Receiving less than 32 bytes before stdin
        /// is closed is considered an error and any bytes after the first 32 are not used for the key
        #[clap(long)]
        key_from_stdin: bool,

        /// If specified, will send output to the specified named pipe (internal usage)
        #[cfg(windows)]
        #[clap(long, help = None, long_help = None)]
        output_to_local_pipe: Option<std::ffi::OsString>,
    },
}

/// Represents the format to use for output from a command.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum Format {
    /// Sends and receives data in JSON format.
    Json,

    /// Commands are traditional shell commands and output responses are inline with what is
    /// expected of a program's output in a shell.
    Shell,
}

impl Format {
    /// Returns true if json format
    pub fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

impl Default for Format {
    fn default() -> Self {
        Self::Shell
    }
}
