#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::builder::TypedValueParser as _;
use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::Shell as ClapCompleteShell;
use derive_more::{Display, Error, From, IsVariant};
use distant_core::net::common::{ConnectionId, Destination, Map, PortRange};
use distant_core::net::server::Shutdown;
use distant_core::protocol::ChangeKind;
use service_manager::ServiceManagerKind;

use crate::constants;
use crate::constants::user::CACHE_FILE_PATH_STR;

mod common;
mod config;

pub use common::*;

pub use self::config::*;

/// Primary entrypoint into options & subcommands for the CLI.
#[derive(Debug, PartialEq, Parser)]
#[clap(author, version, about)]
#[clap(name = "distant")]
pub struct Options {
    #[clap(flatten)]
    pub logging: LoggingSettings,

    /// Configuration file to load instead of the default paths
    #[clap(long = "config", global = true, value_parser)]
    config_path: Option<PathBuf>,

    #[clap(subcommand)]
    pub command: DistantSubcommand,
}

/// Represents an error associated with parsing options.
#[derive(Debug, Display, From, Error)]
pub enum OptionsError {
    // When configuration file fails to load
    Config(#[error(not(source))] anyhow::Error),

    // When parsing options fails (or is something like --version or --help)
    Options(#[error(not(source))] clap::Error),
}

impl Options {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn load() -> Result<Self, OptionsError> {
        Self::load_from(std::env::args_os())
    }

    /// Creates a new CLI instance by parsing providing arguments
    pub fn load_from<I, T>(args: I) -> Result<Self, OptionsError>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut this = Self::try_parse_from(args)?;
        let config = Config::load_multi(this.config_path.take())?;
        this.merge(config);

        // Assign the appropriate log file based on client/manager/server
        if this.logging.log_file.is_none() {
            // NOTE: We assume that any of these commands will log to the user-specific path
            //       and that services that run manager will explicitly override the
            //       log file path
            this.logging.log_file = Some(match &this.command {
                DistantSubcommand::Client(_) => constants::user::CLIENT_LOG_FILE_PATH.to_path_buf(),
                DistantSubcommand::Server(_) => constants::user::SERVER_LOG_FILE_PATH.to_path_buf(),
                DistantSubcommand::Generate(_) => {
                    constants::user::GENERATE_LOG_FILE_PATH.to_path_buf()
                }

                // If we are listening as a manager, then we want to log to a manager-specific file
                DistantSubcommand::Manager(cmd) if cmd.is_listen() => {
                    constants::user::MANAGER_LOG_FILE_PATH.to_path_buf()
                }

                // Otherwise, if we are performing some operation as a client talking to the
                // manager, then we want to log to the client file
                DistantSubcommand::Manager(_) => {
                    constants::user::CLIENT_LOG_FILE_PATH.to_path_buf()
                }
            });
        }

        Ok(this)
    }

    /// Updates options based on configuration values.
    fn merge(&mut self, config: Config) {
        macro_rules! update_logging {
            ($kind:ident) => {{
                self.logging.log_file = self
                    .logging
                    .log_file
                    .take()
                    .or(config.$kind.logging.log_file);
                self.logging.log_level = self.logging.log_level.or(config.$kind.logging.log_level);
            }};
        }

        match &mut self.command {
            DistantSubcommand::Client(cmd) => {
                update_logging!(client);
                match cmd {
                    ClientSubcommand::Api {
                        network, timeout, ..
                    } => {
                        network.merge(config.client.network);
                        *timeout = timeout.take().or(config.client.api.timeout);
                    }
                    ClientSubcommand::Connect {
                        network, options, ..
                    } => {
                        network.merge(config.client.network);
                        options.merge(config.client.connect.options, /* keep */ true);
                    }
                    ClientSubcommand::FileSystem(
                        ClientFileSystemSubcommand::Copy { network, .. }
                        | ClientFileSystemSubcommand::Exists { network, .. }
                        | ClientFileSystemSubcommand::MakeDir { network, .. }
                        | ClientFileSystemSubcommand::Metadata { network, .. }
                        | ClientFileSystemSubcommand::Read { network, .. }
                        | ClientFileSystemSubcommand::Remove { network, .. }
                        | ClientFileSystemSubcommand::Rename { network, .. }
                        | ClientFileSystemSubcommand::Search { network, .. }
                        | ClientFileSystemSubcommand::SetPermissions { network, .. }
                        | ClientFileSystemSubcommand::Watch { network, .. }
                        | ClientFileSystemSubcommand::Write { network, .. },
                    ) => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Launch {
                        distant_args,
                        distant_bin,
                        distant_bind_server,
                        network,
                        options,
                        ..
                    } => {
                        network.merge(config.client.network);
                        options.merge(config.client.launch.options, /* keep */ true);
                        *distant_args = distant_args.take().or(config.client.launch.distant.args);
                        *distant_bin = distant_bin.take().or(config.client.launch.distant.bin);
                        *distant_bind_server =
                            distant_bind_server
                                .take()
                                .or(config.client.launch.distant.bind_server);
                    }
                    ClientSubcommand::Shell { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Spawn { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::SystemInfo { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Version { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Ssh {
                        network, options, ..
                    } => {
                        network.merge(config.client.network);
                        options.merge(config.client.connect.options, /* keep */ true);
                    }
                    ClientSubcommand::Status { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Kill { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Select { network, .. } => {
                        network.merge(config.client.network);
                    }
                }
            }
            DistantSubcommand::Generate(_) => {
                update_logging!(generate);
            }
            DistantSubcommand::Manager(cmd) => {
                update_logging!(manager);
                match cmd {
                    ManagerSubcommand::Version { network, .. } => {
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Listen {
                        access, network, ..
                    } => {
                        *access = access.take().or(config.manager.access);
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Service(_) => (),
                }
            }
            DistantSubcommand::Server(cmd) => {
                update_logging!(server);
                match cmd {
                    ServerSubcommand::Listen {
                        current_dir,
                        host,
                        port,
                        shutdown,
                        use_ipv6,
                        watch,
                        ..
                    } => {
                        //
                        // GENERAL SETTINGS
                        //

                        *current_dir = current_dir.take().or(config.server.listen.current_dir);
                        if host.is_default() {
                            if let Some(host_value) = config.server.listen.host {
                                *host = Value::Explicit(host_value);
                            }
                        }
                        if port.is_default() {
                            if let Some(port_value) = config.server.listen.port {
                                *port = Value::Explicit(port_value);
                            }
                        }
                        if shutdown.is_default() {
                            if let Some(shutdown_value) = config.server.listen.shutdown {
                                *shutdown = Value::Explicit(shutdown_value);
                            }
                        }
                        if !*use_ipv6 && config.server.listen.use_ipv6 {
                            *use_ipv6 = true;
                        }

                        //
                        // WATCH-SPECIFIC SETTINGS
                        //

                        if !watch.watch_polling && !config.server.watch.native {
                            watch.watch_polling = true;
                        }

                        watch.watch_poll_interval = watch
                            .watch_poll_interval
                            .take()
                            .or(config.server.watch.poll_interval);

                        if !watch.watch_compare_contents && config.server.watch.compare_contents {
                            watch.watch_compare_contents = true;
                        }

                        if watch.watch_debounce_timeout.is_default() {
                            if let Some(debounce_timeout) = config.server.watch.debounce_timeout {
                                watch.watch_debounce_timeout = Value::Explicit(debounce_timeout);
                            }
                        }

                        watch.watch_debounce_tick_rate = watch
                            .watch_debounce_tick_rate
                            .take()
                            .or(config.server.watch.debounce_tick_rate);
                    }
                }
            }
        }
    }
}

/// Subcommands for the CLI.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
pub enum DistantSubcommand {
    /// Perform client commands
    #[clap(flatten)]
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

impl DistantSubcommand {
    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        match self {
            Self::Client(x) => x.format(),
            Self::Manager(x) => x.format(),
            Self::Server(x) => x.format(),
            Self::Generate(x) => x.format(),
        }
    }
}

/// Subcommands for `distant client`.
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
pub enum ClientSubcommand {
    /// Listen over stdin & stdout to communicate with a distant server using the JSON lines API
    Api {
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
        timeout: Option<Seconds>,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,
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
        #[clap(long, default_value_t)]
        options: Map,

        #[clap(flatten)]
        network: NetworkSettings,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        /// Force a new connection even if one to the same destination already exists
        #[clap(long)]
        new: bool,

        destination: Box<Destination>,
    },

    /// Subcommands for file system operations
    #[clap(subcommand, name = "fs")]
    FileSystem(ClientFileSystemSubcommand),

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

        /// Path to distant program on remote machine to execute via ssh;
        /// by default, this program needs to be available within PATH as
        /// specified when compiling ssh (not your login shell)
        #[clap(name = "distant", long)]
        distant_bin: Option<String>,

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
        #[clap(name = "distant-bind-server", long, value_name = "ssh|any|IP")]
        distant_bind_server: Option<BindAddress>,

        /// Additional arguments to provide to the server
        #[clap(name = "distant-args", long, allow_hyphen_values(true))]
        distant_args: Option<String>,

        /// Additional options to provide, typically forwarded to the handler within the manager
        /// facilitating the launch of a distant server. Options are key-value pairs separated by
        /// comma.
        ///
        /// E.g. `key="value",key2="value2"`
        #[clap(long, default_value_t)]
        options: Map,

        #[clap(flatten)]
        network: NetworkSettings,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        destination: Box<Destination>,
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
        environment: Map,

        /// Optional command to run instead of $SHELL
        #[clap(name = "CMD", last = true)]
        cmd: Option<Vec<String>>,
    },

    /// Spawn a process on the remote machine
    Spawn {
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

        /// If specified, will assume the remote process is a LSP server
        /// and will translate paths that are local into `distant` and vice versa.
        ///
        /// If a scheme is provided, will translate local paths into that scheme!
        #[clap(long, name = "SCHEME")]
        lsp: Option<Option<String>>,

        /// If specified, will spawn process using a pseudo tty
        #[clap(long)]
        pty: bool,

        /// If specified, will spawn the process in the specified shell, defaulting to the
        /// user-configured shell.
        #[clap(long, name = "SHELL")]
        shell: Option<Option<Shell>>,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// Environment variables to provide to the shell
        #[clap(long, default_value_t)]
        environment: Map,

        /// If present, commands are read from the provided string
        #[clap(short = 'c', long = "cmd", conflicts_with = "CMD")]
        cmd_str: Option<String>,

        /// Command to run
        #[clap(
            name = "CMD",
            num_args = 1..,
            last = true,
            conflicts_with = "cmd_str"
        )]
        cmd: Vec<String>,
    },

    SystemInfo {
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
    },

    /// Retrieves version information of the remote server
    Version {
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

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,
    },

    /// Connect to a remote host via SSH and optionally open a shell or run a command.
    ///
    /// This is the simplest way to use distant. It auto-starts the manager,
    /// connects via SSH (no distant binary needed on the remote), and opens
    /// a shell or runs the specified command.
    ///
    /// Examples:
    ///   distant ssh user@host              # open an interactive shell
    ///   distant ssh user@host -- ls -la    # run a single command
    #[clap(name = "ssh")]
    Ssh {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Additional options to provide to the SSH handler (key-value pairs separated by comma)
        ///
        /// E.g. `key="value",key2="value2"`
        #[clap(long, default_value_t)]
        options: Map,

        #[clap(flatten)]
        network: NetworkSettings,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// Environment variables to provide to the remote shell
        #[clap(long, default_value_t)]
        environment: Map,

        /// Force a new connection even if one to the same destination already exists
        #[clap(long)]
        new: bool,

        /// Destination in the form [user@]host[:port]
        destination: Box<Destination>,

        /// Optional command to run instead of opening an interactive shell
        #[clap(name = "CMD", last = true)]
        cmd: Option<Vec<String>>,
    },

    /// Show the current status of the manager and active connections.
    ///
    /// With no arguments, shows an overview of the manager and all connections.
    /// With a connection ID, shows detailed info about that specific connection.
    Status {
        /// Connection ID to inspect (shows overview if omitted)
        id: Option<ConnectionId>,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

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

    /// Kill an active connection
    Kill {
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        /// Connection to kill (interactive prompt if omitted)
        id: Option<ConnectionId>,

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

    /// Select the active connection
    Select {
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        /// Connection to select (interactive prompt if omitted)
        connection: Option<ConnectionId>,

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
}

impl ClientSubcommand {
    pub fn cache_path(&self) -> &Path {
        match self {
            Self::Connect { cache, .. } => cache.as_path(),
            Self::FileSystem(fs) => fs.cache_path(),
            Self::Launch { cache, .. } => cache.as_path(),
            Self::Api { cache, .. } => cache.as_path(),
            Self::Shell { cache, .. } => cache.as_path(),
            Self::Spawn { cache, .. } => cache.as_path(),
            Self::Ssh { cache, .. } => cache.as_path(),
            Self::Status { cache, .. } => cache.as_path(),
            Self::SystemInfo { cache, .. } => cache.as_path(),
            Self::Version { cache, .. } => cache.as_path(),
            Self::Kill { cache, .. } => cache.as_path(),
            Self::Select { cache, .. } => cache.as_path(),
        }
    }

    pub fn network_settings(&self) -> &NetworkSettings {
        match self {
            Self::Connect { network, .. } => network,
            Self::FileSystem(fs) => fs.network_settings(),
            Self::Launch { network, .. } => network,
            Self::Api { network, .. } => network,
            Self::Shell { network, .. } => network,
            Self::Spawn { network, .. } => network,
            Self::Ssh { network, .. } => network,
            Self::Status { network, .. } => network,
            Self::SystemInfo { network, .. } => network,
            Self::Version { network, .. } => network,
            Self::Kill { network, .. } => network,
            Self::Select { network, .. } => network,
        }
    }

    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        match self {
            Self::Api { .. } => Format::Json,
            Self::Connect { format, .. } => *format,
            Self::FileSystem(fs) => fs.format(),
            Self::Launch { format, .. } => *format,
            Self::Shell { .. } => Format::Shell,
            Self::Spawn { .. } => Format::Shell,
            Self::Ssh { .. } => Format::Shell,
            Self::Status { format, .. } => *format,
            Self::SystemInfo { .. } => Format::Shell,
            Self::Version { format, .. } => *format,
            Self::Kill { format, .. } => *format,
            Self::Select { format, .. } => *format,
        }
    }
}

/// Subcommands for `distant fs`.
#[derive(Debug, PartialEq, Eq, Subcommand, IsVariant)]
pub enum ClientFileSystemSubcommand {
    /// Copies a file or directory on the remote machine
    Copy {
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

        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Checks whether the specified path exists on the remote machine
    Exists {
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

        /// The path to the file or directory on the remote machine
        path: PathBuf,
    },

    /// Creates a directory on the remote machine
    MakeDir {
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

        /// Whether or not to create all parent directories
        #[clap(long)]
        all: bool,

        /// The path to the directory on the remote machine
        path: PathBuf,
    },

    /// Retrieves metadata for the specified path on the remote machine
    Metadata {
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

        /// Whether or not to include a canonicalized version of the path, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        #[clap(long)]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[clap(long)]
        resolve_file_type: bool,

        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,
    },

    /// Reads the contents of a file or retrieves the entries within a directory on the remote
    /// machine
    Read {
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

        /// Maximum depth to traverse with 0 indicating there is no maximum
        /// depth and 1 indicating the most immediate children within the
        /// directory.
        ///
        /// (directory only)
        #[clap(long, default_value_t = 1)]
        depth: usize,

        /// Whether or not to return absolute or relative paths.
        ///
        /// (directory only)
        #[clap(long)]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved.
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true.
        ///
        /// (directory only)
        #[clap(long)]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved entries.
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags.
        ///
        /// (directory only)
        #[clap(long)]
        include_root: bool,

        /// The path to the file or directory on the remote machine.
        path: PathBuf,
    },

    /// Removes a file or directory on the remote machine
    Remove {
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

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[clap(long)]
        force: bool,

        /// The path to the file or directory on the remote machine
        path: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    Rename {
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

        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file or directory
        dst: PathBuf,
    },

    /// Search files & directories on the remote machine
    Search {
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

        /// Kind of data to examine using condition
        #[clap(long, value_enum, default_value_t = CliSearchQueryTarget::Contents)]
        target: CliSearchQueryTarget,

        /// Condition to meet to be considered a match
        #[clap(name = "pattern")]
        condition: CliSearchQueryCondition,

        /// Options to apply to the query
        #[clap(flatten)]
        options: CliSearchQueryOptions,

        /// Paths in which to perform the query
        #[clap(default_value = ".")]
        paths: Vec<PathBuf>,
    },

    /// Sets permissions for the specified path on the remote machine
    SetPermissions {
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

        /// Recursively set permissions of files/directories/symlinks
        #[clap(short = 'R', long)]
        recursive: bool,

        /// Follow symlinks, which means that they will be unaffected
        #[clap(short = 'L', long)]
        follow_symlinks: bool,

        /// Mode string following `chmod` format (or set readonly flag if `readonly` or
        /// `notreadonly` is specified)
        mode: String,

        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,
    },

    /// Watch a path for changes on the remote machine
    Watch {
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

        /// If true, will recursively watch for changes within directories, othewise
        /// will only watch for changes immediately within directories
        #[clap(long)]
        recursive: bool,

        /// Filter to only report back specified changes
        #[
            clap(
                long,
                value_parser = clap::builder::PossibleValuesParser::new(ChangeKind::variants())
                    .map(|s| s.parse::<ChangeKind>().unwrap()),
            )
        ]
        only: Vec<ChangeKind>,

        /// Filter to report back changes except these specified changes
        #[
            clap(
                long,
                value_parser = clap::builder::PossibleValuesParser::new(ChangeKind::variants())
                    .map(|s| s.parse::<ChangeKind>().unwrap()),
            )
        ]
        except: Vec<ChangeKind>,

        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,
    },

    /// Writes the contents to a file on the remote machine
    Write {
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

        /// If specified, will append to a file versus overwriting it
        #[clap(long)]
        append: bool,

        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content. If not provided, will read from stdin.
        data: Option<OsString>,
    },
}

impl ClientFileSystemSubcommand {
    pub fn cache_path(&self) -> &Path {
        match self {
            Self::Copy { cache, .. } => cache.as_path(),
            Self::Exists { cache, .. } => cache.as_path(),
            Self::MakeDir { cache, .. } => cache.as_path(),
            Self::Metadata { cache, .. } => cache.as_path(),
            Self::Read { cache, .. } => cache.as_path(),
            Self::Remove { cache, .. } => cache.as_path(),
            Self::Rename { cache, .. } => cache.as_path(),
            Self::Search { cache, .. } => cache.as_path(),
            Self::SetPermissions { cache, .. } => cache.as_path(),
            Self::Watch { cache, .. } => cache.as_path(),
            Self::Write { cache, .. } => cache.as_path(),
        }
    }

    pub fn network_settings(&self) -> &NetworkSettings {
        match self {
            Self::Copy { network, .. } => network,
            Self::Exists { network, .. } => network,
            Self::MakeDir { network, .. } => network,
            Self::Metadata { network, .. } => network,
            Self::Read { network, .. } => network,
            Self::Remove { network, .. } => network,
            Self::Rename { network, .. } => network,
            Self::Search { network, .. } => network,
            Self::SetPermissions { network, .. } => network,
            Self::Watch { network, .. } => network,
            Self::Write { network, .. } => network,
        }
    }

    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        Format::Shell
    }
}

/// Subcommands for `distant generate`.
#[derive(Debug, PartialEq, Eq, Subcommand, IsVariant)]
pub enum GenerateSubcommand {
    /// Generate configuration file with base settings
    Config {
        /// Write output to a file instead of stdout
        #[clap(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },

    // Generate completion info for CLI
    Completion {
        /// Write output to a file instead of stdout
        #[clap(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Specific shell to target for the generated output
        #[clap(value_enum, value_parser)]
        shell: ClapCompleteShell,
    },
}

impl GenerateSubcommand {
    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        Format::Shell
    }
}

/// Parse a `NAME=PATH` string into a `(String, PathBuf)` for the `--plugin` flag.
fn parse_plugin_flag(s: &str) -> Result<(String, PathBuf), String> {
    match s.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => {
            Ok((name.to_string(), PathBuf::from(path)))
        }
        _ => Err(format!(
            "invalid plugin spec '{s}': expected NAME=PATH (e.g. docker=/usr/local/bin/distant-plugin-docker)"
        )),
    }
}

/// Subcommands for `distant manager`.
#[derive(Debug, PartialEq, Eq, Subcommand, IsVariant)]
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

        /// Register an external plugin (NAME=PATH). Scheme defaults to NAME.
        /// Can be specified multiple times.
        #[clap(long = "plugin", value_parser = parse_plugin_flag)]
        plugin: Vec<(String, PathBuf)>,

        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// Retrieve a list of capabilities that the manager supports
    Version {
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        #[clap(flatten)]
        network: NetworkSettings,
    },
}

impl ManagerSubcommand {
    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        match self {
            Self::Service(_) => Format::Shell,
            Self::Listen { .. } => Format::Shell,
            Self::Version { format, .. } => *format,
        }
    }
}

/// Subcommands for `distant manager service`.
#[derive(Debug, PartialEq, Eq, Subcommand, IsVariant)]
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

        /// Additional arguments to provide to the manager when started
        #[clap(name = "ARGS", last = true)]
        args: Vec<String>,
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
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
pub enum ServerSubcommand {
    /// Listen for incoming requests as a server
    Listen {
        /// Control the IP address that the distant binds to
        ///
        /// There are three options here:
        ///
        /// 1. `ssh`: the server will reply from the IP address that the SSH
        ///    connection came from (as found in the SSH_CONNECTION environment variable). This is
        ///    useful for multihomed servers.
        ///
        /// 2. `any`: the server will reply on the default interface and will not bind to
        ///    a particular IP address. This can be useful if the connection is made through ssh or
        ///    another tool that makes the SSH connection appear to come from localhost.
        ///
        /// 3. `IP`: the server will attempt to bind to the specified IP address.
        #[clap(long, value_name = "ssh|any|IP", default_value_t = Value::Default(BindAddress::Any))]
        host: Value<BindAddress>,

        /// Set the port(s) that the server will attempt to bind to
        ///
        /// This can be in the form of PORT1 or PORT1:PORTN to provide a range of ports.
        /// With `--port 0`, the server will let the operating system pick an available TCP port.
        ///
        /// Please note that this option does not affect the server-side port used by SSH
        #[clap(long, value_name = "PORT[:PORT2]", default_value_t = Value::Default(PortRange::EPHEMERAL))]
        port: Value<PortRange>,

        /// If specified, will bind to the ipv6 interface if host is "any" instead of ipv4
        #[clap(short = '6', long)]
        use_ipv6: bool,

        /// Logic to apply to server when determining when to shutdown automatically
        ///
        /// 1. "never" means the server will never automatically shut down
        /// 2. "after=<N>" means the server will shut down after N seconds
        /// 3. "lonely=<N>" means the server will shut down after N seconds with no connections
        ///
        /// Default is to never shut down
        #[clap(long, default_value_t = Value::Default(Shutdown::Never))]
        shutdown: Value<Shutdown>,

        /// Changes the current working directory (cwd) to the specified directory
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

        #[clap(flatten)]
        watch: ServerListenWatchOptions,

        /// If specified, the server will not generate a key but instead listen on stdin for the next
        /// 32 bytes that it will use as the key instead. Receiving less than 32 bytes before stdin
        /// is closed is considered an error and any bytes after the first 32 are not used for the key
        #[clap(long)]
        key_from_stdin: bool,

        /// If specified, will send output to the specified named pipe (internal usage)
        #[clap(long, help = None, long_help = None)]
        output_to_local_pipe: Option<std::ffi::OsString>,
    },
}

impl ServerSubcommand {
    /// Format used by the subcommand.
    #[inline]
    pub fn format(&self) -> Format {
        Format::Shell
    }
}

#[derive(Args, Debug, PartialEq)]
pub struct ServerListenWatchOptions {
    /// If specified, will use the polling-based watcher for filesystem changes
    #[clap(long)]
    pub watch_polling: bool,

    /// If specified, represents the time (in seconds) between polls of files being watched,
    /// only relevant when using the polling watcher implementation
    #[clap(long)]
    pub watch_poll_interval: Option<Seconds>,

    /// If true, will attempt to load a file and compare its contents to detect file changes,
    /// only relevant when using the polling watcher implementation (VERY SLOW)
    #[clap(long)]
    pub watch_compare_contents: bool,

    /// Represents the maximum time (in seconds) to wait for filesystem changes before
    /// reporting them, which is useful to avoid noisy changes as well as serves to consolidate
    /// different events that represent the same action
    #[clap(long, default_value_t = Value::Default(Seconds::try_from(0.5).unwrap()))]
    pub watch_debounce_timeout: Value<Seconds>,

    /// Represents how often (in seconds) to check for new events before the debounce timeout
    /// occurs. Defaults to 1/4 the debounce timeout if not set.
    #[clap(long)]
    pub watch_debounce_tick_rate: Option<Seconds>,
}

/// Represents the format to use for output from a command.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[derive(Default)]
pub enum Format {
    /// Sends and receives data in JSON format.
    Json,

    /// Commands are traditional shell commands and output responses are inline with what is
    /// expected of a program's output in a shell.
    #[default]
    Shell,
}

impl Format {
    /// Returns true if json format
    pub fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

#[cfg(test)]
mod tests {
    //! Comprehensive tests for `Options` and all sub-command types: `format()`,
    //! `cache_path()`, `network_settings()` accessors, `parse_plugin_flag`,
    //! `Format` enum, `IsVariant` derives, config merge behavior, and CLI
    //! argument parsing.

    use std::time::Duration;

    use distant_core::map;
    use distant_core::net::common::Host;

    use super::*;

    #[test]
    fn distant_api_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Api {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                timeout: None,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                api: ClientApiConfig {
                    timeout: Some(Seconds::from(5u32)),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Api {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    timeout: Some(Seconds::from(5u32)),
                }),
            }
        );
    }

    #[test]
    fn distant_api_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Api {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                timeout: Some(Seconds::from(99u32)),
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                api: ClientApiConfig {
                    timeout: Some(Seconds::from(5u32)),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Api {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    timeout: Some(Seconds::from(99u32)),
                }),
            }
        );
    }

    #[test]
    fn distant_capabilities_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Version {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                format: Format::Json,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("hello" -> "world"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Version {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    format: Format::Json,
                }),
            }
        );
    }

    #[test]
    fn distant_capabilities_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Version {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                format: Format::Json,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("hello" -> "world", "config" -> "value"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Version {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    format: Format::Json,
                }),
            }
        );
    }

    #[test]
    fn distant_connect_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Connect {
                cache: PathBuf::new(),
                options: map!(),
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                format: Format::Json,
                new: false,
                destination: Box::new("test://destination".parse().unwrap()),
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("hello" -> "world"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Connect {
                    cache: PathBuf::new(),
                    options: map!("hello" -> "world"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    format: Format::Json,
                    new: false,
                    destination: Box::new("test://destination".parse().unwrap()),
                }),
            }
        );
    }

    #[test]
    fn distant_connect_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Connect {
                cache: PathBuf::new(),
                options: map!("hello" -> "test", "cli" -> "value"),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                format: Format::Json,
                new: false,
                destination: Box::new("test://destination".parse().unwrap()),
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("hello" -> "world", "config" -> "value"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Connect {
                    cache: PathBuf::new(),
                    options: map!("hello" -> "test", "cli" -> "value", "config" -> "value"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    format: Format::Json,
                    new: false,
                    destination: Box::new("test://destination".parse().unwrap()),
                }),
            }
        );
    }

    #[test]
    fn distant_launch_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Launch {
                cache: PathBuf::new(),
                distant_bin: None,
                distant_bind_server: None,
                distant_args: None,
                options: map!(),
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                format: Format::Json,
                destination: Box::new("test://destination".parse().unwrap()),
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                launch: ClientLaunchConfig {
                    distant: ClientLaunchDistantConfig {
                        args: Some(String::from("config-args")),
                        bin: Some(String::from("config-bin")),
                        bind_server: Some(BindAddress::Host(Host::Name(String::from(
                            "config-host",
                        )))),
                    },
                    options: map!("hello" -> "world"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Launch {
                    cache: PathBuf::new(),
                    distant_args: Some(String::from("config-args")),
                    distant_bin: Some(String::from("config-bin")),
                    distant_bind_server: Some(BindAddress::Host(Host::Name(String::from(
                        "config-host",
                    )))),
                    options: map!("hello" -> "world"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    format: Format::Json,
                    destination: Box::new("test://destination".parse().unwrap()),
                }),
            }
        );
    }

    #[test]
    fn distant_launch_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Launch {
                cache: PathBuf::new(),
                distant_args: Some(String::from("cli-args")),
                distant_bin: Some(String::from("cli-bin")),
                distant_bind_server: Some(BindAddress::Host(Host::Name(String::from("cli-host")))),
                options: map!("hello" -> "test", "cli" -> "value"),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                format: Format::Json,
                destination: Box::new("test://destination".parse().unwrap()),
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                launch: ClientLaunchConfig {
                    distant: ClientLaunchDistantConfig {
                        args: Some(String::from("config-args")),
                        bin: Some(String::from("config-bin")),
                        bind_server: Some(BindAddress::Host(Host::Name(String::from(
                            "config-host",
                        )))),
                    },
                    options: map!("hello" -> "world", "config" -> "value"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Launch {
                    cache: PathBuf::new(),
                    distant_args: Some(String::from("cli-args")),
                    distant_bin: Some(String::from("cli-bin")),
                    distant_bind_server: Some(BindAddress::Host(Host::Name(String::from(
                        "cli-host",
                    )))),
                    options: map!("hello" -> "test", "config" -> "value", "cli" -> "value"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    format: Format::Json,
                    destination: Box::new("test://destination".parse().unwrap()),
                }),
            }
        );
    }

    #[test]
    fn distant_shell_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Shell {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                current_dir: None,
                environment: Default::default(),
                cmd: None,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Shell {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    current_dir: None,
                    environment: map!(),
                    cmd: None,
                }),
            }
        );
    }

    #[test]
    fn distant_shell_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Shell {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                current_dir: None,
                environment: map!(),
                cmd: None,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Shell {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    current_dir: None,
                    environment: map!(),
                    cmd: None,
                }),
            }
        );
    }

    #[test]
    fn distant_spawn_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Spawn {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                current_dir: None,
                environment: map!(),
                lsp: Some(None),
                shell: Some(None),
                pty: true,
                cmd_str: None,
                cmd: vec![String::from("cmd")],
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Spawn {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    current_dir: None,
                    environment: map!(),
                    lsp: Some(None),
                    shell: Some(None),
                    pty: true,
                    cmd_str: None,
                    cmd: vec![String::from("cmd")],
                }),
            }
        );
    }

    #[test]
    fn distant_spawn_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Spawn {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                current_dir: None,
                environment: map!(),
                lsp: Some(None),
                shell: Some(None),
                pty: true,
                cmd_str: None,
                cmd: vec![String::from("cmd")],
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Spawn {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    current_dir: None,
                    environment: map!(),
                    lsp: Some(None),
                    shell: Some(None),
                    pty: true,
                    cmd_str: None,
                    cmd: vec![String::from("cmd")],
                }),
            }
        );
    }

    #[test]
    fn distant_system_info_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::SystemInfo {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::SystemInfo {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_system_info_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::SystemInfo {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::SystemInfo {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_fs_copy_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Copy {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Copy {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        src: PathBuf::from("src"),
                        dst: PathBuf::from("dst"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_copy_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Copy {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Copy {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        src: PathBuf::from("src"),
                        dst: PathBuf::from("dst"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_exists_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Exists {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Exists {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_exists_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Exists {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Exists {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_makedir_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::MakeDir {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    path: PathBuf::from("path"),
                    all: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::MakeDir {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        all: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_makedir_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::MakeDir {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    path: PathBuf::from("path"),
                    all: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::MakeDir {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        all: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_metadata_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Metadata {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    canonicalize: true,
                    resolve_file_type: true,
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Metadata {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        canonicalize: true,
                        resolve_file_type: true,
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_metadata_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Metadata {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    canonicalize: true,
                    resolve_file_type: true,
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Metadata {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        canonicalize: true,
                        resolve_file_type: true,
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_read_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Read {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    path: PathBuf::from("path"),
                    depth: 1,
                    absolute: true,
                    canonicalize: true,
                    include_root: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Read {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        depth: 1,
                        absolute: true,
                        canonicalize: true,
                        include_root: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_read_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Read {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    path: PathBuf::from("path"),
                    depth: 1,
                    absolute: true,
                    canonicalize: true,
                    include_root: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Read {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        depth: 1,
                        absolute: true,
                        canonicalize: true,
                        include_root: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_remove_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Remove {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    path: PathBuf::from("path"),
                    force: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Remove {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        force: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_remove_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Remove {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    path: PathBuf::from("path"),
                    force: true,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Remove {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        path: PathBuf::from("path"),
                        force: true,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_rename_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Rename {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Rename {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        src: PathBuf::from("src"),
                        dst: PathBuf::from("dst"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_rename_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Rename {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Rename {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        src: PathBuf::from("src"),
                        dst: PathBuf::from("dst"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_search_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Search {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    target: CliSearchQueryTarget::Contents,
                    condition: CliSearchQueryCondition::regex(".*"),
                    options: Default::default(),
                    paths: vec![PathBuf::from(".")],
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Search {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        target: CliSearchQueryTarget::Contents,
                        condition: CliSearchQueryCondition::regex(".*"),
                        options: Default::default(),
                        paths: vec![PathBuf::from(".")],
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_search_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Search {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    target: CliSearchQueryTarget::Contents,
                    condition: CliSearchQueryCondition::regex(".*"),
                    options: Default::default(),
                    paths: vec![PathBuf::from(".")],
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Search {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        target: CliSearchQueryTarget::Contents,
                        condition: CliSearchQueryCondition::regex(".*"),
                        options: Default::default(),
                        paths: vec![PathBuf::from(".")],
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_watch_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Watch {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    recursive: true,
                    only: ChangeKind::all(),
                    except: ChangeKind::all(),
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Watch {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        recursive: true,
                        only: ChangeKind::all(),
                        except: ChangeKind::all(),
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_watch_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Watch {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    recursive: true,
                    only: ChangeKind::all(),
                    except: ChangeKind::all(),
                    path: PathBuf::from("path"),
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Watch {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        recursive: true,
                        only: ChangeKind::all(),
                        except: ChangeKind::all(),
                        path: PathBuf::from("path"),
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_write_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Write {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None,
                    },
                    append: false,
                    path: PathBuf::from("path"),
                    data: None,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Write {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("config-unix-socket")),
                            windows_pipe: Some(String::from("config-windows-pipe")),
                        },
                        append: false,
                        path: PathBuf::from("path"),
                        data: None,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_fs_write_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                ClientFileSystemSubcommand::Write {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    append: false,
                    path: PathBuf::from("path"),
                    data: None,
                },
            )),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::FileSystem(
                    ClientFileSystemSubcommand::Write {
                        cache: PathBuf::new(),
                        connection: None,
                        network: NetworkSettings {
                            unix_socket: Some(PathBuf::from("cli-unix-socket")),
                            windows_pipe: Some(String::from("cli-windows-pipe")),
                        },
                        append: false,
                        path: PathBuf::from("path"),
                        data: None,
                    }
                )),
            }
        );
    }

    #[test]
    fn distant_generate_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Generate(GenerateSubcommand::Completion {
                output: None,
                shell: ClapCompleteShell::Bash,
            }),
        };

        options.merge(Config {
            generate: GenerateConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Generate(GenerateSubcommand::Completion {
                    output: None,
                    shell: ClapCompleteShell::Bash,
                }),
            }
        );
    }

    #[test]
    fn distant_generate_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Generate(GenerateSubcommand::Completion {
                output: None,
                shell: ClapCompleteShell::Bash,
            }),
        };

        options.merge(Config {
            generate: GenerateConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Generate(GenerateSubcommand::Completion {
                    output: None,
                    shell: ClapCompleteShell::Bash,
                }),
            }
        );
    }

    #[test]
    fn distant_manager_capabilities_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Version {
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            manager: ManagerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Version {
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_manager_capabilities_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Version {
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            manager: ManagerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Version {
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_status_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Status {
                id: None,
                format: Format::Shell,
                cache: PathBuf::new(),
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Status {
                    id: None,
                    format: Format::Shell,
                    cache: PathBuf::new(),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_status_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Status {
                id: Some(0),
                format: Format::Json,
                cache: PathBuf::new(),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Status {
                    id: Some(0),
                    format: Format::Json,
                    cache: PathBuf::new(),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_kill_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Kill {
                cache: PathBuf::new(),
                id: Some(0),
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Kill {
                    cache: PathBuf::new(),
                    id: Some(0),
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_kill_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Kill {
                cache: PathBuf::new(),
                id: Some(0),
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Kill {
                    cache: PathBuf::new(),
                    id: Some(0),
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_select_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Select {
                cache: PathBuf::new(),
                connection: None,
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Select {
                    cache: PathBuf::new(),
                    connection: None,
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_select_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Select {
                cache: PathBuf::new(),
                connection: None,
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Select {
                    cache: PathBuf::new(),
                    connection: None,
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_manager_listen_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Listen {
                access: None,
                daemon: false,
                user: false,
                plugin: Vec::new(),
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
            }),
        };

        options.merge(Config {
            manager: ManagerConfig {
                access: Some(AccessControl::Group),
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Listen {
                    access: Some(AccessControl::Group),
                    daemon: false,
                    user: false,
                    plugin: Vec::new(),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_manager_listen_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Listen {
                access: Some(AccessControl::Owner),
                daemon: false,
                user: false,
                plugin: Vec::new(),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
            }),
        };

        options.merge(Config {
            manager: ManagerConfig {
                access: Some(AccessControl::Group),
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Listen {
                    access: Some(AccessControl::Owner),
                    daemon: false,
                    user: false,
                    plugin: Vec::new(),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                }),
            }
        );
    }

    #[test]
    fn distant_manager_service_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Install {
                    kind: None,
                    user: false,
                    args: Vec::new(),
                },
            )),
        };

        options.merge(Config {
            manager: ManagerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Service(
                    ManagerServiceSubcommand::Install {
                        kind: None,
                        user: false,
                        args: Vec::new(),
                    },
                )),
            }
        );
    }

    #[test]
    fn distant_manager_service_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Install {
                    kind: None,
                    user: false,
                    args: Vec::new(),
                },
            )),
        };

        options.merge(Config {
            manager: ManagerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Manager(ManagerSubcommand::Service(
                    ManagerServiceSubcommand::Install {
                        kind: None,
                        user: false,
                        args: Vec::new(),
                    },
                )),
            }
        );
    }

    #[test]
    fn distant_server_listen_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Server(ServerSubcommand::Listen {
                host: Value::Default(BindAddress::Any),
                port: Value::Default(PortRange::single(123)),
                use_ipv6: false,
                shutdown: Value::Default(Shutdown::After(Duration::from_secs(123))),
                current_dir: None,
                watch: ServerListenWatchOptions {
                    watch_polling: false,
                    watch_poll_interval: None,
                    watch_compare_contents: false,
                    watch_debounce_timeout: Value::Default(Seconds::try_from(0.5).unwrap()),
                    watch_debounce_tick_rate: None,
                },
                daemon: false,
                key_from_stdin: false,
                output_to_local_pipe: None,
            }),
        };

        options.merge(Config {
            server: ServerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                listen: ServerListenConfig {
                    host: Some(BindAddress::Ssh),
                    port: Some(PortRange::single(456)),
                    use_ipv6: true,
                    shutdown: Some(Shutdown::Lonely(Duration::from_secs(456))),
                    current_dir: Some(PathBuf::from("config-dir")),
                },
                watch: ServerWatchConfig {
                    native: false,
                    poll_interval: Some(Seconds::from(100u32)),
                    compare_contents: true,
                    debounce_timeout: Some(Seconds::from(200u32)),
                    debounce_tick_rate: Some(Seconds::from(300u32)),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Server(ServerSubcommand::Listen {
                    host: Value::Explicit(BindAddress::Ssh),
                    port: Value::Explicit(PortRange::single(456)),
                    use_ipv6: true,
                    shutdown: Value::Explicit(Shutdown::Lonely(Duration::from_secs(456))),
                    current_dir: Some(PathBuf::from("config-dir")),
                    watch: ServerListenWatchOptions {
                        watch_polling: true,
                        watch_poll_interval: Some(Seconds::from(100u32)),
                        watch_compare_contents: true,
                        watch_debounce_timeout: Value::Explicit(Seconds::from(200u32)),
                        watch_debounce_tick_rate: Some(Seconds::from(300u32)),
                    },
                    daemon: false,
                    key_from_stdin: false,
                    output_to_local_pipe: None,
                }),
            }
        );
    }

    #[test]
    fn distant_server_listen_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Server(ServerSubcommand::Listen {
                host: Value::Explicit(BindAddress::Any),
                port: Value::Explicit(PortRange::single(123)),
                use_ipv6: true,
                shutdown: Value::Explicit(Shutdown::After(Duration::from_secs(123))),
                current_dir: Some(PathBuf::from("cli-dir")),
                watch: ServerListenWatchOptions {
                    watch_polling: true,
                    watch_poll_interval: Some(Seconds::from(10u32)),
                    watch_compare_contents: true,
                    watch_debounce_timeout: Value::Explicit(Seconds::from(20u32)),
                    watch_debounce_tick_rate: Some(Seconds::from(30u32)),
                },
                daemon: false,
                key_from_stdin: false,
                output_to_local_pipe: None,
            }),
        };

        options.merge(Config {
            server: ServerConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                listen: ServerListenConfig {
                    host: Some(BindAddress::Ssh),
                    port: Some(PortRange::single(456)),
                    use_ipv6: false,
                    shutdown: Some(Shutdown::Lonely(Duration::from_secs(456))),
                    current_dir: Some(PathBuf::from("config-dir")),
                },
                watch: ServerWatchConfig {
                    native: true,
                    poll_interval: Some(Seconds::from(100u32)),
                    compare_contents: false,
                    debounce_timeout: Some(Seconds::from(200u32)),
                    debounce_tick_rate: Some(Seconds::from(300u32)),
                },
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Server(ServerSubcommand::Listen {
                    host: Value::Explicit(BindAddress::Any),
                    port: Value::Explicit(PortRange::single(123)),
                    use_ipv6: true,
                    shutdown: Value::Explicit(Shutdown::After(Duration::from_secs(123))),
                    current_dir: Some(PathBuf::from("cli-dir")),
                    watch: ServerListenWatchOptions {
                        watch_polling: true,
                        watch_poll_interval: Some(Seconds::from(10u32)),
                        watch_compare_contents: true,
                        watch_debounce_timeout: Value::Explicit(Seconds::from(20u32)),
                        watch_debounce_tick_rate: Some(Seconds::from(30u32)),
                    },
                    daemon: false,
                    key_from_stdin: false,
                    output_to_local_pipe: None,
                }),
            }
        );
    }

    // -----------------------------------------------------------------------
    // Clap parsing tests for new/moved top-level commands
    // -----------------------------------------------------------------------

    #[test]
    fn distant_status_should_parse_without_id() {
        let options = Options::try_parse_from(["distant", "status"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Status { id, .. }) => {
                assert_eq!(id, None);
            }
            other => panic!("Expected Status, got {other:?}"),
        }
    }

    #[test]
    fn distant_status_should_parse_with_id() {
        let options = Options::try_parse_from(["distant", "status", "5"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Status { id, .. }) => {
                assert_eq!(id, Some(5));
            }
            other => panic!("Expected Status with id=5, got {other:?}"),
        }
    }

    #[test]
    fn distant_status_should_parse_with_format_json() {
        let options = Options::try_parse_from(["distant", "status", "--format", "json"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Status { format, .. }) => {
                assert_eq!(format, Format::Json);
            }
            other => panic!("Expected Status with json format, got {other:?}"),
        }
    }

    #[test]
    fn distant_kill_should_parse_without_id() {
        let options = Options::try_parse_from(["distant", "kill"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Kill { id, .. }) => {
                assert_eq!(id, None);
            }
            other => panic!("Expected Kill, got {other:?}"),
        }
    }

    #[test]
    fn distant_kill_should_parse_with_id() {
        let options = Options::try_parse_from(["distant", "kill", "5"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Kill { id, .. }) => {
                assert_eq!(id, Some(5));
            }
            other => panic!("Expected Kill with id=5, got {other:?}"),
        }
    }

    #[test]
    fn distant_select_should_parse_without_connection() {
        let options = Options::try_parse_from(["distant", "select"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Select { connection, .. }) => {
                assert_eq!(connection, None);
            }
            other => panic!("Expected Select, got {other:?}"),
        }
    }

    #[test]
    fn distant_select_should_parse_with_connection() {
        let options = Options::try_parse_from(["distant", "select", "5"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Select { connection, .. }) => {
                assert_eq!(connection, Some(5));
            }
            other => panic!("Expected Select with connection=5, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_list_should_not_parse() {
        assert!(Options::try_parse_from(["distant", "manager", "list"]).is_err());
    }

    #[test]
    fn distant_manager_kill_should_not_parse() {
        assert!(Options::try_parse_from(["distant", "manager", "kill"]).is_err());
    }

    #[test]
    fn distant_manager_info_should_not_parse() {
        assert!(Options::try_parse_from(["distant", "manager", "info"]).is_err());
    }

    #[test]
    fn distant_manager_select_should_not_parse() {
        assert!(Options::try_parse_from(["distant", "manager", "select"]).is_err());
    }

    // -------------------------------------------------------
    // format() method tests for DistantSubcommand and children
    // -------------------------------------------------------
    #[test]
    fn format_api_returns_json() {
        let cmd = ClientSubcommand::Api {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            timeout: None,
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_shell_returns_shell() {
        let cmd = ClientSubcommand::Shell {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            current_dir: None,
            environment: Default::default(),
            cmd: None,
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn format_spawn_returns_shell() {
        let cmd = ClientSubcommand::Spawn {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            current_dir: None,
            environment: Default::default(),
            lsp: None,
            shell: None,
            pty: false,
            cmd_str: None,
            cmd: vec![],
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn format_ssh_returns_shell() {
        let cmd = ClientSubcommand::Ssh {
            cache: PathBuf::new(),
            options: Default::default(),
            network: NetworkSettings::default(),
            current_dir: None,
            environment: Default::default(),
            new: false,
            destination: Box::new("test://host".parse().unwrap()),
            cmd: None,
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn format_system_info_returns_shell() {
        let cmd = ClientSubcommand::SystemInfo {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn format_connect_returns_specified_format() {
        let cmd = ClientSubcommand::Connect {
            cache: PathBuf::new(),
            options: Default::default(),
            network: NetworkSettings::default(),
            format: Format::Json,
            new: false,
            destination: Box::new("test://host".parse().unwrap()),
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_launch_returns_specified_format() {
        let cmd = ClientSubcommand::Launch {
            cache: PathBuf::new(),
            distant_bin: None,
            distant_bind_server: None,
            distant_args: None,
            options: Default::default(),
            network: NetworkSettings::default(),
            format: Format::Shell,
            destination: Box::new("test://host".parse().unwrap()),
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn format_status_returns_specified_format() {
        let cmd = ClientSubcommand::Status {
            id: None,
            format: Format::Json,
            network: NetworkSettings::default(),
            cache: PathBuf::new(),
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_version_returns_specified_format() {
        let cmd = ClientSubcommand::Version {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            format: Format::Json,
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_kill_returns_specified_format() {
        let cmd = ClientSubcommand::Kill {
            format: Format::Json,
            id: None,
            network: NetworkSettings::default(),
            cache: PathBuf::new(),
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_select_returns_specified_format() {
        let cmd = ClientSubcommand::Select {
            format: Format::Json,
            connection: None,
            network: NetworkSettings::default(),
            cache: PathBuf::new(),
        };
        assert!(cmd.format().is_json());
    }

    #[test]
    fn format_filesystem_returns_shell() {
        let cmd = ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Copy {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
        });
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn distant_subcommand_format_delegates_correctly() {
        let sub = DistantSubcommand::Client(ClientSubcommand::Api {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            timeout: None,
        });
        assert!(sub.format().is_json());

        let sub = DistantSubcommand::Generate(GenerateSubcommand::Config { output: None });
        assert_eq!(sub.format(), Format::Shell);

        let sub = DistantSubcommand::Server(ServerSubcommand::Listen {
            host: Value::Default(BindAddress::Any),
            port: Value::Default(distant_core::net::common::PortRange::EPHEMERAL),
            use_ipv6: false,
            shutdown: Value::Default(distant_core::net::server::Shutdown::Never),
            current_dir: None,
            daemon: false,
            watch: ServerListenWatchOptions {
                watch_polling: false,
                watch_poll_interval: None,
                watch_compare_contents: false,
                watch_debounce_timeout: Value::Default(Seconds::try_from(0.5).unwrap()),
                watch_debounce_tick_rate: None,
            },
            key_from_stdin: false,
            output_to_local_pipe: None,
        });
        assert_eq!(sub.format(), Format::Shell);

        let sub = DistantSubcommand::Manager(ManagerSubcommand::Listen {
            access: None,
            daemon: false,
            user: false,
            plugin: Vec::new(),
            network: NetworkSettings::default(),
        });
        assert_eq!(sub.format(), Format::Shell);

        let sub = DistantSubcommand::Manager(ManagerSubcommand::Version {
            format: Format::Json,
            network: NetworkSettings::default(),
        });
        assert!(sub.format().is_json());

        let sub = DistantSubcommand::Manager(ManagerSubcommand::Service(
            ManagerServiceSubcommand::Start {
                kind: None,
                user: false,
            },
        ));
        assert_eq!(sub.format(), Format::Shell);
    }

    // -------------------------------------------------------
    // cache_path() method tests
    // -------------------------------------------------------
    #[test]
    fn cache_path_returns_correct_path_for_each_client_subcommand() {
        let cache = PathBuf::from("/test/cache");
        let net = NetworkSettings::default();

        let cases: Vec<ClientSubcommand> = vec![
            ClientSubcommand::Api {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                timeout: None,
            },
            ClientSubcommand::Connect {
                cache: cache.clone(),
                options: Default::default(),
                network: net.clone(),
                format: Format::Shell,
                new: false,
                destination: Box::new("test://host".parse().unwrap()),
            },
            ClientSubcommand::Launch {
                cache: cache.clone(),
                distant_bin: None,
                distant_bind_server: None,
                distant_args: None,
                options: Default::default(),
                network: net.clone(),
                format: Format::Shell,
                destination: Box::new("test://host".parse().unwrap()),
            },
            ClientSubcommand::Shell {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                current_dir: None,
                environment: Default::default(),
                cmd: None,
            },
            ClientSubcommand::Spawn {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                current_dir: None,
                environment: Default::default(),
                lsp: None,
                shell: None,
                pty: false,
                cmd_str: None,
                cmd: vec![],
            },
            ClientSubcommand::SystemInfo {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
            },
            ClientSubcommand::Version {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                format: Format::Shell,
            },
            ClientSubcommand::Ssh {
                cache: cache.clone(),
                options: Default::default(),
                network: net.clone(),
                current_dir: None,
                environment: Default::default(),
                new: false,
                destination: Box::new("test://host".parse().unwrap()),
                cmd: None,
            },
            ClientSubcommand::Status {
                id: None,
                format: Format::Shell,
                network: net.clone(),
                cache: cache.clone(),
            },
            ClientSubcommand::Kill {
                format: Format::Shell,
                id: None,
                network: net.clone(),
                cache: cache.clone(),
            },
            ClientSubcommand::Select {
                format: Format::Shell,
                connection: None,
                network: net.clone(),
                cache: cache.clone(),
            },
        ];

        for cmd in &cases {
            assert_eq!(
                cmd.cache_path(),
                cache.as_path(),
                "cache_path failed for {:?}",
                std::mem::discriminant(cmd)
            );
        }
    }

    #[test]
    fn cache_path_filesystem_subcommands() {
        let cache = PathBuf::from("/test/cache");
        let net = NetworkSettings::default();

        let fs_cases: Vec<ClientFileSystemSubcommand> = vec![
            ClientFileSystemSubcommand::Copy {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                src: PathBuf::from("a"),
                dst: PathBuf::from("b"),
            },
            ClientFileSystemSubcommand::Exists {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::MakeDir {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                all: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Metadata {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                canonicalize: false,
                resolve_file_type: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Read {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Remove {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                force: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Rename {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                src: PathBuf::from("a"),
                dst: PathBuf::from("b"),
            },
            ClientFileSystemSubcommand::Search {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                target: CliSearchQueryTarget::Contents,
                condition: CliSearchQueryCondition::regex("test"),
                options: Default::default(),
                paths: vec![],
            },
            ClientFileSystemSubcommand::SetPermissions {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                recursive: false,
                follow_symlinks: false,
                mode: String::from("644"),
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Watch {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                recursive: false,
                only: vec![],
                except: vec![],
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Write {
                cache: cache.clone(),
                connection: None,
                network: net.clone(),
                append: false,
                path: PathBuf::from("a"),
                data: None,
            },
        ];

        for fs_cmd in &fs_cases {
            assert_eq!(
                fs_cmd.cache_path(),
                cache.as_path(),
                "cache_path failed for filesystem {:?}",
                std::mem::discriminant(fs_cmd)
            );
        }

        // Also test through ClientSubcommand::FileSystem wrapper
        let cmd = ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Copy {
            cache: cache.clone(),
            connection: None,
            network: net.clone(),
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
        });
        assert_eq!(cmd.cache_path(), cache.as_path());
    }

    // -------------------------------------------------------
    // network_settings() method tests
    // -------------------------------------------------------
    #[test]
    fn network_settings_returns_correct_ref_for_each_client_subcommand() {
        let net = NetworkSettings {
            unix_socket: Some(PathBuf::from("/test/socket")),
            windows_pipe: Some(String::from("test-pipe")),
        };

        let cmd = ClientSubcommand::Api {
            cache: PathBuf::new(),
            connection: None,
            network: net.clone(),
            timeout: None,
        };
        assert_eq!(cmd.network_settings(), &net);

        let cmd = ClientSubcommand::Ssh {
            cache: PathBuf::new(),
            options: Default::default(),
            network: net.clone(),
            current_dir: None,
            environment: Default::default(),
            new: false,
            destination: Box::new("test://host".parse().unwrap()),
            cmd: None,
        };
        assert_eq!(cmd.network_settings(), &net);

        let cmd = ClientSubcommand::Status {
            id: None,
            format: Format::Shell,
            network: net.clone(),
            cache: PathBuf::new(),
        };
        assert_eq!(cmd.network_settings(), &net);

        let cmd = ClientSubcommand::Kill {
            format: Format::Shell,
            id: None,
            network: net.clone(),
            cache: PathBuf::new(),
        };
        assert_eq!(cmd.network_settings(), &net);

        let cmd = ClientSubcommand::Select {
            format: Format::Shell,
            connection: None,
            network: net.clone(),
            cache: PathBuf::new(),
        };
        assert_eq!(cmd.network_settings(), &net);

        // FileSystem wrapper
        let cmd = ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Copy {
            cache: PathBuf::new(),
            connection: None,
            network: net.clone(),
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
        });
        assert_eq!(cmd.network_settings(), &net);
    }

    #[test]
    fn network_settings_filesystem_subcommands() {
        let net = NetworkSettings {
            unix_socket: Some(PathBuf::from("/test/socket")),
            windows_pipe: Some(String::from("test-pipe")),
        };

        let fs_cmds: Vec<ClientFileSystemSubcommand> = vec![
            ClientFileSystemSubcommand::Exists {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::MakeDir {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                all: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Metadata {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                canonicalize: false,
                resolve_file_type: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Read {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Remove {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                force: false,
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Rename {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                src: PathBuf::from("a"),
                dst: PathBuf::from("b"),
            },
            ClientFileSystemSubcommand::Search {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                target: CliSearchQueryTarget::Contents,
                condition: CliSearchQueryCondition::regex("test"),
                options: Default::default(),
                paths: vec![],
            },
            ClientFileSystemSubcommand::SetPermissions {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                recursive: false,
                follow_symlinks: false,
                mode: String::from("644"),
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Watch {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                recursive: false,
                only: vec![],
                except: vec![],
                path: PathBuf::from("a"),
            },
            ClientFileSystemSubcommand::Write {
                cache: PathBuf::new(),
                connection: None,
                network: net.clone(),
                append: false,
                path: PathBuf::from("a"),
                data: None,
            },
        ];

        for fs_cmd in &fs_cmds {
            assert_eq!(
                fs_cmd.network_settings(),
                &net,
                "network_settings failed for {:?}",
                std::mem::discriminant(fs_cmd)
            );
        }
    }

    // -------------------------------------------------------
    // parse_plugin_flag tests
    // -------------------------------------------------------
    #[test]
    fn parse_plugin_flag_valid() {
        let (name, path) =
            parse_plugin_flag("docker=/usr/local/bin/distant-plugin-docker").unwrap();
        assert_eq!(name, "docker");
        assert_eq!(path, PathBuf::from("/usr/local/bin/distant-plugin-docker"));
    }

    #[test]
    fn parse_plugin_flag_simple() {
        let (name, path) = parse_plugin_flag("myplugin=./plugin").unwrap();
        assert_eq!(name, "myplugin");
        assert_eq!(path, PathBuf::from("./plugin"));
    }

    #[test]
    fn parse_plugin_flag_empty_name_fails() {
        assert!(parse_plugin_flag("=/some/path").is_err());
    }

    #[test]
    fn parse_plugin_flag_empty_path_fails() {
        assert!(parse_plugin_flag("name=").is_err());
    }

    #[test]
    fn parse_plugin_flag_no_equals_fails() {
        assert!(parse_plugin_flag("noequalssign").is_err());
    }

    #[test]
    fn parse_plugin_flag_empty_string_fails() {
        assert!(parse_plugin_flag("").is_err());
    }

    // -------------------------------------------------------
    // Format::is_json tests
    // -------------------------------------------------------
    #[test]
    fn format_is_json_returns_true_for_json() {
        assert!(Format::Json.is_json());
    }

    #[test]
    fn format_is_json_returns_false_for_shell() {
        assert!(!Format::Shell.is_json());
    }

    #[test]
    fn format_default_is_shell() {
        assert_eq!(Format::default(), Format::Shell);
    }

    // -------------------------------------------------------
    // ManagerSubcommand::is_listen / IsVariant
    // -------------------------------------------------------
    #[test]
    fn manager_subcommand_is_listen_returns_true_for_listen() {
        let cmd = ManagerSubcommand::Listen {
            access: None,
            daemon: false,
            user: false,
            plugin: Vec::new(),
            network: NetworkSettings::default(),
        };
        assert!(cmd.is_listen());
    }

    #[test]
    fn manager_subcommand_is_listen_returns_false_for_version() {
        let cmd = ManagerSubcommand::Version {
            format: Format::Shell,
            network: NetworkSettings::default(),
        };
        assert!(!cmd.is_listen());
    }

    #[test]
    fn manager_subcommand_is_listen_returns_false_for_service() {
        let cmd = ManagerSubcommand::Service(ManagerServiceSubcommand::Start {
            kind: None,
            user: false,
        });
        assert!(!cmd.is_listen());
    }

    // -------------------------------------------------------
    // GenerateSubcommand::format tests
    // -------------------------------------------------------
    #[test]
    fn generate_config_format_is_shell() {
        let cmd = GenerateSubcommand::Config { output: None };
        assert_eq!(cmd.format(), Format::Shell);
    }

    #[test]
    fn generate_completion_format_is_shell() {
        let cmd = GenerateSubcommand::Completion {
            output: None,
            shell: ClapCompleteShell::Bash,
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    // -------------------------------------------------------
    // ServerSubcommand::format tests
    // -------------------------------------------------------
    #[test]
    fn server_listen_format_is_shell() {
        let cmd = ServerSubcommand::Listen {
            host: Value::Default(BindAddress::Any),
            port: Value::Default(distant_core::net::common::PortRange::EPHEMERAL),
            use_ipv6: false,
            shutdown: Value::Default(distant_core::net::server::Shutdown::Never),
            current_dir: None,
            daemon: false,
            watch: ServerListenWatchOptions {
                watch_polling: false,
                watch_poll_interval: None,
                watch_compare_contents: false,
                watch_debounce_timeout: Value::Default(Seconds::try_from(0.5).unwrap()),
                watch_debounce_tick_rate: None,
            },
            key_from_stdin: false,
            output_to_local_pipe: None,
        };
        assert_eq!(cmd.format(), Format::Shell);
    }

    // -------------------------------------------------------
    // Ssh merge tests
    // -------------------------------------------------------
    #[test]
    fn distant_ssh_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Ssh {
                cache: PathBuf::new(),
                options: map!(),
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                current_dir: None,
                environment: Default::default(),
                new: false,
                destination: Box::new("test://host".parse().unwrap()),
                cmd: None,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("hello" -> "world"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Ssh {
                    cache: PathBuf::new(),
                    options: map!("hello" -> "world"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    current_dir: None,
                    environment: Default::default(),
                    new: false,
                    destination: Box::new("test://host".parse().unwrap()),
                    cmd: None,
                }),
            }
        );
    }

    #[test]
    fn distant_ssh_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Ssh {
                cache: PathBuf::new(),
                options: map!("cli_key" -> "cli_val"),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                current_dir: None,
                environment: Default::default(),
                new: false,
                destination: Box::new("test://host".parse().unwrap()),
                cmd: None,
            }),
        };

        options.merge(Config {
            client: ClientConfig {
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("config-log-file")),
                    log_level: Some(LogLevel::Trace),
                },
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("config-unix-socket")),
                    windows_pipe: Some(String::from("config-windows-pipe")),
                },
                connect: ClientConnectConfig {
                    options: map!("cli_key" -> "config_val", "config_key" -> "config_val"),
                },
                ..Default::default()
            },
            ..Default::default()
        });

        assert_eq!(
            options,
            Options {
                config_path: None,
                logging: LoggingSettings {
                    log_file: Some(PathBuf::from("cli-log-file")),
                    log_level: Some(LogLevel::Info),
                },
                command: DistantSubcommand::Client(ClientSubcommand::Ssh {
                    cache: PathBuf::new(),
                    options: map!("cli_key" -> "cli_val", "config_key" -> "config_val"),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    current_dir: None,
                    environment: Default::default(),
                    new: false,
                    destination: Box::new("test://host".parse().unwrap()),
                    cmd: None,
                }),
            }
        );
    }

    // -------------------------------------------------------
    // CLI parsing tests for subcommands
    // -------------------------------------------------------
    #[test]
    fn distant_ssh_should_parse_basic() {
        let options = Options::try_parse_from(["distant", "ssh", "user@host"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Ssh {
                destination,
                cmd,
                new,
                ..
            }) => {
                assert_eq!(destination.host.to_string(), "host");
                assert!(cmd.is_none());
                assert!(!new);
            }
            other => panic!("Expected Ssh, got {other:?}"),
        }
    }

    #[test]
    fn distant_ssh_should_parse_with_command() {
        let options =
            Options::try_parse_from(["distant", "ssh", "user@host", "--", "ls", "-la"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Ssh { cmd, .. }) => {
                assert_eq!(cmd, Some(vec!["ls".to_string(), "-la".to_string()]));
            }
            other => panic!("Expected Ssh, got {other:?}"),
        }
    }

    #[test]
    fn distant_ssh_should_parse_with_new_flag() {
        let options = Options::try_parse_from(["distant", "ssh", "--new", "user@host"]).unwrap();
        match options.command {
            DistantSubcommand::Client(ClientSubcommand::Ssh { new, .. }) => {
                assert!(new);
            }
            other => panic!("Expected Ssh with --new, got {other:?}"),
        }
    }

    #[test]
    fn distant_server_listen_should_parse_defaults() {
        let options = Options::try_parse_from(["distant", "server", "listen"]).unwrap();
        match options.command {
            DistantSubcommand::Server(ServerSubcommand::Listen {
                use_ipv6,
                daemon,
                key_from_stdin,
                ..
            }) => {
                assert!(!use_ipv6);
                assert!(!daemon);
                assert!(!key_from_stdin);
            }
            other => panic!("Expected Server Listen, got {other:?}"),
        }
    }

    #[test]
    fn distant_server_listen_should_parse_with_flags() {
        let options = Options::try_parse_from([
            "distant",
            "server",
            "listen",
            "--use-ipv6",
            "--daemon",
            "--key-from-stdin",
        ])
        .unwrap();
        match options.command {
            DistantSubcommand::Server(ServerSubcommand::Listen {
                use_ipv6,
                daemon,
                key_from_stdin,
                ..
            }) => {
                assert!(use_ipv6);
                assert!(daemon);
                assert!(key_from_stdin);
            }
            other => panic!("Expected Server Listen with flags, got {other:?}"),
        }
    }

    #[test]
    fn distant_generate_config_should_parse() {
        let options = Options::try_parse_from(["distant", "generate", "config"]).unwrap();
        match options.command {
            DistantSubcommand::Generate(GenerateSubcommand::Config { output }) => {
                assert!(output.is_none());
            }
            other => panic!("Expected Generate Config, got {other:?}"),
        }
    }

    #[test]
    fn distant_generate_completion_should_parse() {
        let options =
            Options::try_parse_from(["distant", "generate", "completion", "bash"]).unwrap();
        match options.command {
            DistantSubcommand::Generate(GenerateSubcommand::Completion { output, shell }) => {
                assert!(output.is_none());
                assert_eq!(shell, ClapCompleteShell::Bash);
            }
            other => panic!("Expected Generate Completion, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_listen_should_parse_with_plugin() {
        let options = Options::try_parse_from([
            "distant",
            "manager",
            "listen",
            "--plugin",
            "docker=/usr/local/bin/docker-plugin",
        ])
        .unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Listen { plugin, .. }) => {
                assert_eq!(plugin.len(), 1);
                assert_eq!(plugin[0].0, "docker");
                assert_eq!(plugin[0].1, PathBuf::from("/usr/local/bin/docker-plugin"));
            }
            other => panic!("Expected Manager Listen with plugin, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_listen_should_parse_with_multiple_plugins() {
        let options = Options::try_parse_from([
            "distant",
            "manager",
            "listen",
            "--plugin",
            "a=/path/a",
            "--plugin",
            "b=/path/b",
        ])
        .unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Listen { plugin, .. }) => {
                assert_eq!(plugin.len(), 2);
            }
            other => panic!("Expected Manager Listen with plugins, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_service_install_should_parse() {
        let options =
            Options::try_parse_from(["distant", "manager", "service", "install", "--user"])
                .unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Install { user, kind, args },
            )) => {
                assert!(user);
                assert!(kind.is_none());
                assert!(args.is_empty());
            }
            other => panic!("Expected Manager Service Install, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_service_uninstall_should_parse() {
        let options =
            Options::try_parse_from(["distant", "manager", "service", "uninstall"]).unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Uninstall { user, kind },
            )) => {
                assert!(!user);
                assert!(kind.is_none());
            }
            other => panic!("Expected Manager Service Uninstall, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_service_start_should_parse() {
        let options = Options::try_parse_from(["distant", "manager", "service", "start"]).unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Start { user, kind },
            )) => {
                assert!(!user);
                assert!(kind.is_none());
            }
            other => panic!("Expected Manager Service Start, got {other:?}"),
        }
    }

    #[test]
    fn distant_manager_service_stop_should_parse() {
        let options = Options::try_parse_from(["distant", "manager", "service", "stop"]).unwrap();
        match options.command {
            DistantSubcommand::Manager(ManagerSubcommand::Service(
                ManagerServiceSubcommand::Stop { user, kind },
            )) => {
                assert!(!user);
                assert!(kind.is_none());
            }
            other => panic!("Expected Manager Service Stop, got {other:?}"),
        }
    }

    // -------------------------------------------------------
    // ManagerServiceSubcommand IsVariant
    // -------------------------------------------------------
    #[test]
    fn manager_service_subcommand_is_variant() {
        let start = ManagerServiceSubcommand::Start {
            kind: None,
            user: false,
        };
        assert!(start.is_start());
        assert!(!start.is_stop());
        assert!(!start.is_install());
        assert!(!start.is_uninstall());

        let stop = ManagerServiceSubcommand::Stop {
            kind: None,
            user: false,
        };
        assert!(stop.is_stop());
    }

    // -------------------------------------------------------
    // DistantSubcommand IsVariant
    // -------------------------------------------------------
    #[test]
    fn distant_subcommand_is_variant() {
        let client = DistantSubcommand::Client(ClientSubcommand::Api {
            cache: PathBuf::new(),
            connection: None,
            network: NetworkSettings::default(),
            timeout: None,
        });
        assert!(client.is_client());
        assert!(!client.is_server());
        assert!(!client.is_manager());
        assert!(!client.is_generate());
    }
}
