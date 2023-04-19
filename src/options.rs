use crate::constants;
use crate::constants::user::CACHE_FILE_PATH_STR;
use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::Shell as ClapCompleteShell;
use derive_more::IsVariant;
use distant_core::data::{DistantRequestData, Environment};
use distant_core::net::common::{ConnectionId, Destination, Map, PortRange};
use distant_core::net::server::Shutdown;
use service_manager::ServiceManagerKind;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod common;
mod config;

pub use self::config::*;
pub use common::*;

/// Primary entrypoint into options & subcommands for the CLI.
#[derive(Debug, PartialEq, Parser)]
#[clap(author, version, about)]
#[clap(name = "distant")]
pub struct Options {
    #[clap(flatten)]
    pub logging: LoggingSettings,

    /// Configuration file to load instead of the default paths
    #[clap(short = 'c', long = "config", global = true, value_parser)]
    config_path: Option<PathBuf>,

    #[clap(subcommand)]
    pub command: DistantSubcommand,
}

impl Options {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(std::env::args_os())
    }

    /// Creates a new CLI instance by parsing providing arguments
    pub fn load_from<I, T>(args: I) -> anyhow::Result<Self>
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
                    ClientSubcommand::Action {
                        network, timeout, ..
                    } => {
                        network.merge(config.client.network);
                        *timeout = timeout.take().or(config.client.action.timeout);
                    }
                    ClientSubcommand::Connect {
                        network, options, ..
                    } => {
                        network.merge(config.client.network);
                        options.merge(config.client.connect.options, /* keep */ true);
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
                    ClientSubcommand::Lsp { network, .. } => {
                        network.merge(config.client.network);
                    }
                    ClientSubcommand::Repl {
                        network, timeout, ..
                    } => {
                        network.merge(config.client.network);
                        *timeout = timeout.take().or(config.client.repl.timeout);
                    }
                    ClientSubcommand::Shell { network, .. } => {
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
                    ManagerSubcommand::Capabilities { network, .. } => {
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Info { network, .. } => {
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Kill { network, .. } => {
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::List { network, .. } => {
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Listen {
                        access, network, ..
                    } => {
                        *access = access.take().or(config.manager.access);
                        network.merge(config.manager.network);
                    }
                    ManagerSubcommand::Select { network, .. } => {
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
                        ..
                    } => {
                        *current_dir = current_dir.take().or(config.server.listen.current_dir);
                        if host.is_default() && config.server.listen.host.is_some() {
                            *host = Value::Explicit(config.server.listen.host.unwrap());
                        }
                        if port.is_default() && config.server.listen.port.is_some() {
                            *port = Value::Explicit(config.server.listen.port.unwrap());
                        }
                        if shutdown.is_default() && config.server.listen.shutdown.is_some() {
                            *shutdown = Value::Explicit(config.server.listen.shutdown.unwrap());
                        }
                        if !*use_ipv6 && config.server.listen.use_ipv6 {
                            *use_ipv6 = true;
                        }
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

/// Subcommands for `distant client`.
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
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
        #[clap(long, default_value_t)]
        options: Map,

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

        /// Represents the maximum time (in seconds) to wait for a network request before timing out.
        #[clap(long)]
        timeout: Option<f32>,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkSettings,

        /// Format used for input into and output from the repl
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,
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

impl ClientSubcommand {
    pub fn cache_path(&self) -> &Path {
        match self {
            Self::Action { cache, .. } => cache.as_path(),
            Self::Connect { cache, .. } => cache.as_path(),
            Self::Launch { cache, .. } => cache.as_path(),
            Self::Lsp { cache, .. } => cache.as_path(),
            Self::Repl { cache, .. } => cache.as_path(),
            Self::Shell { cache, .. } => cache.as_path(),
        }
    }

    pub fn network_settings(&self) -> &NetworkSettings {
        match self {
            Self::Action { network, .. } => network,
            Self::Connect { network, .. } => network,
            Self::Launch { network, .. } => network,
            Self::Lsp { network, .. } => network,
            Self::Repl { network, .. } => network,
            Self::Shell { network, .. } => network,
        }
    }
}

/// Subcommands for `distant generate`.
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
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
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
pub enum ManagerSubcommand {
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
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// Retrieve information about a specific connection
    Info {
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        id: ConnectionId,

        #[clap(flatten)]
        network: NetworkSettings,
    },

    /// List information about all connections
    List {
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

    /// Kill a specific connection
    Kill {
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        #[clap(flatten)]
        network: NetworkSettings,

        id: ConnectionId,
    },
}

/// Subcommands for `distant manager service`.
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
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
#[derive(Debug, PartialEq, Subcommand, IsVariant)]
pub enum ServerSubcommand {
    /// Listen for incoming requests as a server
    Listen {
        /// Control the IP address that the distant binds to
        ///
        /// There are three options here:
        ///
        /// 1. `ssh`: the server will reply from the IP address that the SSH
        /// connection came from (as found in the SSH_CONNECTION environment variable). This is
        /// useful for multihomed servers.
        ///
        /// 2. `any`: the server will reply on the default interface and will not bind to
        /// a particular IP address. This can be useful if the connection is made through ssh or
        /// another tool that makes the SSH connection appear to come from localhost.
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

#[cfg(test)]
mod tests {
    use super::*;
    use distant_core::net::common::Host;
    use distant_core::net::map;

    #[test]
    fn distant_action_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Action {
                cache: PathBuf::new(),
                connection: None,
                timeout: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                request: DistantRequestData::SystemInfo {},
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
                action: ClientActionConfig { timeout: Some(5.0) },
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
                command: DistantSubcommand::Client(ClientSubcommand::Action {
                    cache: PathBuf::new(),
                    connection: None,
                    timeout: Some(5.0),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    request: DistantRequestData::SystemInfo {},
                }),
            }
        );
    }

    #[test]
    fn distant_action_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Action {
                cache: PathBuf::new(),
                connection: None,
                timeout: Some(99.0),
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                request: DistantRequestData::SystemInfo {},
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
                action: ClientActionConfig { timeout: Some(5.0) },
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
                command: DistantSubcommand::Client(ClientSubcommand::Action {
                    cache: PathBuf::new(),
                    connection: None,
                    timeout: Some(99.0),
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    request: DistantRequestData::SystemInfo {},
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
    fn distant_lsp_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Lsp {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: None,
                    windows_pipe: None,
                },
                current_dir: None,
                pty: false,
                cmd: String::from("cmd"),
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
                command: DistantSubcommand::Client(ClientSubcommand::Lsp {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    current_dir: None,
                    pty: false,
                    cmd: String::from("cmd"),
                }),
            }
        );
    }

    #[test]
    fn distant_lsp_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Lsp {
                cache: PathBuf::new(),
                connection: None,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                current_dir: None,
                pty: false,
                cmd: String::from("cmd"),
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
                command: DistantSubcommand::Client(ClientSubcommand::Lsp {
                    cache: PathBuf::new(),
                    connection: None,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    current_dir: None,
                    pty: false,
                    cmd: String::from("cmd"),
                }),
            }
        );
    }

    #[test]
    fn distant_repl_should_support_merging_with_config() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: None,
                log_level: None,
            },
            command: DistantSubcommand::Client(ClientSubcommand::Repl {
                cache: PathBuf::new(),
                connection: None,
                format: Format::Json,
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
                repl: ClientReplConfig { timeout: Some(5.0) },
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
                command: DistantSubcommand::Client(ClientSubcommand::Repl {
                    cache: PathBuf::new(),
                    connection: None,
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("config-unix-socket")),
                        windows_pipe: Some(String::from("config-windows-pipe")),
                    },
                    timeout: Some(5.0),
                }),
            }
        );
    }

    #[test]
    fn distant_repl_should_prioritize_explicit_cli_options_when_merging() {
        let mut options = Options {
            config_path: None,
            logging: LoggingSettings {
                log_file: Some(PathBuf::from("cli-log-file")),
                log_level: Some(LogLevel::Info),
            },
            command: DistantSubcommand::Client(ClientSubcommand::Repl {
                cache: PathBuf::new(),
                connection: None,
                format: Format::Json,
                network: NetworkSettings {
                    unix_socket: Some(PathBuf::from("cli-unix-socket")),
                    windows_pipe: Some(String::from("cli-windows-pipe")),
                },
                timeout: Some(99.0),
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
                repl: ClientReplConfig { timeout: Some(5.0) },
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
                command: DistantSubcommand::Client(ClientSubcommand::Repl {
                    cache: PathBuf::new(),
                    connection: None,
                    format: Format::Json,
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("cli-unix-socket")),
                        windows_pipe: Some(String::from("cli-windows-pipe")),
                    },
                    timeout: Some(99.0),
                }),
            }
        );
    }

    #[test]
    fn distant_shell_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_shell_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_generate_should_support_merging_with_config() {
        todo!("Test logging override");
    }

    #[test]
    fn distant_generate_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
    }

    #[test]
    fn distant_manager_capabilities_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_capabilities_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_info_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_info_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_kill_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_kill_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_list_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_list_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_listen_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
        todo!("Test access control override");
    }

    #[test]
    fn distant_manager_listen_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
        todo!("Test access control override");
    }

    #[test]
    fn distant_manager_select_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_select_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test network override");
    }

    #[test]
    fn distant_manager_service_should_support_merging_with_config() {
        todo!("Test logging override");
    }

    #[test]
    fn distant_manager_service_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
    }

    #[test]
    fn distant_server_should_support_merging_with_config() {
        todo!("Test logging override");
        todo!("Test current-dir override");
        todo!("Test host override");
        todo!("Test port override");
        todo!("Test shutdown override");
        todo!("Test use-ipv6 override");
    }

    #[test]
    fn distant_server_should_prioritize_explicit_cli_options_when_merging() {
        todo!("Test logging override");
        todo!("Test current-dir override");
        todo!("Test host override");
        todo!("Test port override");
        todo!("Test shutdown override");
        todo!("Test use-ipv6 override");
    }
}
