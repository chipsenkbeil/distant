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
}
