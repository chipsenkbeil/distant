use crate::{
    cli::{Cache, Client, Manager},
    config::{AccessControl, ManagerConfig, NetworkConfig},
    paths::user::CACHE_FILE_PATH_STR,
    CliResult,
};
use anyhow::Context;
use clap::{Subcommand, ValueHint};
use distant_core::net::common::ConnectionId;
use distant_core::net::manager::{Config as NetManagerConfig, ConnectHandler, LaunchHandler};
use log::*;
use once_cell::sync::Lazy;
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceManagerKind,
    ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::{collections::HashMap, ffi::OsString, path::PathBuf};
use tabled::{Table, Tabled};

/// [`ServiceLabel`] for our manager in the form `rocks.distant.manager`
static SERVICE_LABEL: Lazy<ServiceLabel> = Lazy::new(|| ServiceLabel {
    qualifier: String::from("rocks"),
    organization: String::from("distant"),
    application: String::from("manager"),
});

mod handlers;

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
        network: NetworkConfig,
    },

    /// Retrieve a list of capabilities that the manager supports
    Capabilities {
        #[clap(flatten)]
        network: NetworkConfig,
    },

    /// Retrieve information about a specific connection
    Info {
        id: ConnectionId,
        #[clap(flatten)]
        network: NetworkConfig,
    },

    /// List information about all connections
    List {
        #[clap(flatten)]
        network: NetworkConfig,

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
        network: NetworkConfig,
        id: ConnectionId,
    },

    /// Send a shutdown request to the manager
    Shutdown {
        #[clap(flatten)]
        network: NetworkConfig,
    },
}

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

impl ManagerSubcommand {
    /// Returns true if the manager subcommand is listen
    pub fn is_listen(&self) -> bool {
        matches!(self, Self::Listen { .. })
    }

    pub fn run(self, config: ManagerConfig) -> CliResult {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self, config),
            _ => {
                let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
                rt.block_on(Self::async_run(self, config))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self, _config: ManagerConfig) -> CliResult {
        use crate::cli::Spawner;
        let pid = Spawner::spawn_running_background(Vec::new())
            .context("Failed to spawn background process")?;
        println!("[distant manager detached, pid = {}]", pid);
        Ok(())
    }

    #[cfg(unix)]
    fn run_daemon(self, config: ManagerConfig) -> CliResult {
        use crate::CliError;
        use fork::{daemon, Fork};

        debug!("Forking process");
        match daemon(true, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
                rt.block_on(async { Self::async_run(self, config).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                println!("[distant manager detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(CliError::Error(anyhow::anyhow!("Fork failed to close fd")))
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(CliError::Error(anyhow::anyhow!("Fork failed"))),
        }
    }

    async fn async_run(self, config: ManagerConfig) -> CliResult {
        match self {
            Self::Service(ManagerServiceSubcommand::Start { kind, user }) => {
                debug!("Starting manager service via {:?}", kind);
                let mut manager = <dyn ServiceManager>::target_or_native(kind)
                    .context("Failed to detect native service manager")?;

                if user {
                    manager
                        .set_level(ServiceLevel::User)
                        .context("Failed to set service manager to user level")?;
                }

                manager
                    .start(ServiceStartCtx {
                        label: SERVICE_LABEL.clone(),
                    })
                    .context("Failed to start service")?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Stop { kind, user }) => {
                debug!("Stopping manager service via {:?}", kind);
                let mut manager = <dyn ServiceManager>::target_or_native(kind)
                    .context("Failed to detect native service manager")?;

                if user {
                    manager
                        .set_level(ServiceLevel::User)
                        .context("Failed to set service manager to user level")?;
                }

                manager
                    .stop(ServiceStopCtx {
                        label: SERVICE_LABEL.clone(),
                    })
                    .context("Failed to stop service")?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Install { kind, user }) => {
                debug!("Installing manager service via {:?}", kind);
                let mut manager = <dyn ServiceManager>::target_or_native(kind)
                    .context("Failed to detect native service manager")?;
                let mut args = vec![OsString::from("manager"), OsString::from("listen")];

                if user {
                    args.push(OsString::from("--user"));
                    manager
                        .set_level(ServiceLevel::User)
                        .context("Failed to set service manager to user level")?;
                }

                manager
                    .install(ServiceInstallCtx {
                        label: SERVICE_LABEL.clone(),

                        // distant manager listen
                        program: std::env::current_exe()
                            .ok()
                            .unwrap_or_else(|| PathBuf::from("distant")),
                        args,
                    })
                    .context("Failed to install service")?;

                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Uninstall { kind, user }) => {
                debug!("Uninstalling manager service via {:?}", kind);
                let mut manager = <dyn ServiceManager>::target_or_native(kind)
                    .context("Failed to detect native service manager")?;
                if user {
                    manager
                        .set_level(ServiceLevel::User)
                        .context("Failed to set service manager to user level")?;
                }
                manager
                    .uninstall(ServiceUninstallCtx {
                        label: SERVICE_LABEL.clone(),
                    })
                    .context("Failed to uninstall service")?;

                Ok(())
            }
            Self::Listen {
                access,
                network,
                user,
                ..
            } => {
                let access = access.or(config.access).unwrap_or_default();
                let network = network.merge(config.network);

                info!(
                    "Starting manager (network = {})",
                    if (cfg!(windows) && network.windows_pipe.is_some())
                        || (cfg!(unix) && network.unix_socket.is_some())
                    {
                        "custom"
                    } else if user {
                        "user"
                    } else {
                        "global"
                    }
                );
                let manager_ref = Manager {
                    access,
                    config: NetManagerConfig {
                        user,
                        launch_handlers: {
                            let mut handlers: HashMap<String, Box<dyn LaunchHandler>> =
                                HashMap::new();
                            handlers.insert(
                                "manager".to_string(),
                                Box::new(handlers::ManagerLaunchHandler::new()),
                            );

                            #[cfg(any(feature = "libssh", feature = "ssh2"))]
                            handlers
                                .insert("ssh".to_string(), Box::new(handlers::SshLaunchHandler));

                            handlers
                        },
                        connect_handlers: {
                            let mut handlers: HashMap<String, Box<dyn ConnectHandler>> =
                                HashMap::new();

                            handlers.insert(
                                "distant".to_string(),
                                Box::new(handlers::DistantConnectHandler),
                            );

                            #[cfg(any(feature = "libssh", feature = "ssh2"))]
                            handlers
                                .insert("ssh".to_string(), Box::new(handlers::SshConnectHandler));

                            handlers
                        },
                        ..Default::default()
                    },
                    network,
                }
                .listen()
                .await
                .context("Failed to start manager")?;

                // Let our server run to completion
                manager_ref
                    .as_ref()
                    .polling_wait()
                    .await
                    .context("Failed to wait on manager")?;
                info!("Manager is shutting down");

                Ok(())
            }
            Self::Capabilities { network } => {
                let network = network.merge(config.network);
                debug!("Getting list of capabilities");
                let caps = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?
                    .capabilities()
                    .await
                    .context("Failed to get list of capabilities")?;
                debug!("Got capabilities: {caps:?}");

                #[derive(Tabled)]
                struct CapabilityRow {
                    kind: String,
                    description: String,
                }

                println!(
                    "{}",
                    Table::new(caps.into_sorted_vec().into_iter().map(|cap| {
                        CapabilityRow {
                            kind: cap.kind,
                            description: cap.description,
                        }
                    }))
                );

                Ok(())
            }
            Self::Info { network, id } => {
                let network = network.merge(config.network);
                debug!("Getting info about connection {}", id);
                let info = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?
                    .info(id)
                    .await
                    .context("Failed to get info about connection")?;
                debug!("Got info: {info:?}");

                #[derive(Tabled)]
                struct InfoRow {
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                    options: String,
                }

                println!(
                    "{}",
                    Table::new(vec![InfoRow {
                        id: info.id,
                        scheme: info.destination.scheme.unwrap_or_default(),
                        host: info.destination.host.to_string(),
                        port: info
                            .destination
                            .port
                            .map(|x| x.to_string())
                            .unwrap_or_default(),
                        options: info.options.to_string()
                    }])
                );

                Ok(())
            }
            Self::List { network, cache } => {
                let network = network.merge(config.network);
                debug!("Getting list of connections");
                let list = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?
                    .list()
                    .await
                    .context("Failed to get list of connections")?;
                debug!("Got list: {list:?}");

                debug!("Looking up selected connection");
                let selected = Cache::read_from_disk_or_default(cache)
                    .await
                    .context("Failed to look up selected connection")?
                    .data
                    .selected;
                debug!("Using selected: {selected}");

                #[derive(Tabled)]
                struct ListRow {
                    selected: bool,
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                }

                println!(
                    "{}",
                    Table::new(list.into_iter().map(|(id, destination)| {
                        ListRow {
                            selected: *selected == id,
                            id,
                            scheme: destination.scheme.unwrap_or_default(),
                            host: destination.host.to_string(),
                            port: destination.port.map(|x| x.to_string()).unwrap_or_default(),
                        }
                    }))
                );

                Ok(())
            }
            Self::Kill { network, id } => {
                let network = network.merge(config.network);
                debug!("Killing connection {}", id);
                Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?
                    .kill(id)
                    .await
                    .with_context(|| format!("Failed to kill connection to server {id}"))?;
                debug!("Connection killed");
                Ok(())
            }
            Self::Shutdown { network } => {
                let network = network.merge(config.network);
                debug!("Shutting down manager");
                Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?
                    .shutdown()
                    .await
                    .context("Failed to shutdown manager")?;
                Ok(())
            }
        }
    }
}
