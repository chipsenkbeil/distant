use crate::{
    cli::{
        CliResult, Client, Manager, Service, ServiceInstallCtx, ServiceKind, ServiceLabel,
        ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx,
    },
    config::{ManagerConfig, NetworkConfig},
    paths::user as user_paths,
};
use clap::Subcommand;
use distant_core::{net::ServerRef, ConnectionId, DistantManagerConfig};
use log::*;
use once_cell::sync::Lazy;
use std::io;
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
    #[clap(subcommand)]
    Service(ManagerServiceSubcommand),

    /// Listen for incoming requests as a manager
    Listen {
        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

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
        kind: Option<ServiceKind>,
    },

    /// Stop the manager as a service
    Stop {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,
    },

    /// Install the manager as a service
    Install {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,

        /// If specified, installs as a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Uninstall the manager as a service
    Uninstall {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,

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

    pub fn run(self, config: ManagerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self, config),
            _ => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(Self::async_run(self, config))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self, config: ManagerConfig) -> CliResult<()> {
        use crate::cli::Spawner;
        let pid = Spawner::spawn_running_background(Vec::new())?;
        println!("[distant manager detached, pid = {}]", pid);
        Ok(())
    }

    #[cfg(unix)]
    fn run_daemon(self, config: ManagerConfig) -> CliResult<()> {
        use fork::{daemon, Fork};

        debug!("Forking process");
        match daemon(true, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { Self::async_run(self, config).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                println!("[distant manager detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into())
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Fork failed").into()),
        }
    }

    async fn async_run(self, config: ManagerConfig) -> CliResult<()> {
        match self {
            Self::Service(ManagerServiceSubcommand::Start { kind }) => {
                debug!("Starting manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.start(ServiceStartCtx {
                    label: SERVICE_LABEL.clone(),
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Stop { kind }) => {
                debug!("Stopping manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.stop(ServiceStopCtx {
                    label: SERVICE_LABEL.clone(),
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Install { kind, user }) => {
                debug!("Installing manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                let mut args = vec!["manager".to_string(), "listen".to_string()];

                // Add pointer to user-specific path for unix socket or name for windows named pipe
                if user {
                    #[cfg(unix)]
                    {
                        args.push("--unix-socket".to_string());
                        args.push(format!("{:?}", user_paths::UNIX_SOCKET_PATH));
                    }

                    #[cfg(windows)]
                    {
                        args.push("--windows-pipe".to_string());
                        args.push(user_paths::WINDOWS_PIPE_NAME.to_string());
                    }
                }

                service.install(ServiceInstallCtx {
                    label: SERVICE_LABEL.clone(),
                    user,

                    // distant manager listen
                    program: std::env::current_exe()
                        .ok()
                        .and_then(|p| p.to_str().map(ToString::to_string))
                        .unwrap_or_else(|| String::from("distant")),
                    args,
                })?;

                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Uninstall { kind, user }) => {
                debug!("Uninstalling manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.uninstall(ServiceUninstallCtx {
                    label: SERVICE_LABEL.clone(),
                    user,
                })?;

                Ok(())
            }
            Self::Listen { network, .. } => {
                let network = network.merge(config.network);
                info!("Starting manager");
                let manager_ref = Manager::new(DistantManagerConfig::default(), network)
                    .listen()
                    .await?;

                // Register our handlers for different schemes
                debug!("Registering handlers with manager");
                manager_ref
                    .register_launch_handler("manager", handlers::ManagerLaunchHandler)
                    .await?;
                manager_ref
                    .register_launch_handler("ssh", handlers::SshLaunchHandler)
                    .await?;
                manager_ref
                    .register_connect_handler("distant", handlers::DistantConnectHandler)
                    .await?;
                manager_ref
                    .register_connect_handler("ssh", handlers::SshConnectHandler)
                    .await?;

                // Let our server run to completion
                manager_ref.wait().await?;
                info!("Manager is shutting down");

                Ok(())
            }
            Self::Info { network, id } => {
                let network = network.merge(config.network);
                debug!("Getting info about connection {}", id);
                let info = Client::new(network).connect().await?.info(id).await?;

                #[derive(Tabled)]
                struct InfoRow {
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                    extra: String,
                }

                println!(
                    "{}",
                    Table::new(vec![InfoRow {
                        id: info.id,
                        scheme: info
                            .destination
                            .scheme()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                        host: info.destination.to_host_string(),
                        port: info
                            .destination
                            .port()
                            .map(|x| x.to_string())
                            .unwrap_or_default(),
                        extra: info.extra.to_string()
                    }])
                );

                Ok(())
            }
            Self::List { network } => {
                let network = network.merge(config.network);
                debug!("Getting list of connections");
                let list = Client::new(network).connect().await?.list().await?;

                #[derive(Tabled)]
                struct ListRow {
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                }

                println!(
                    "{}",
                    Table::new(list.into_iter().map(|(id, destination)| {
                        ListRow {
                            id,
                            scheme: destination
                                .scheme()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                            host: destination.to_host_string(),
                            port: destination
                                .port()
                                .map(|x| x.to_string())
                                .unwrap_or_default(),
                        }
                    }))
                );

                Ok(())
            }
            Self::Kill { network, id } => {
                let network = network.merge(config.network);
                debug!("Killing connection {}", id);
                Client::new(network).connect().await?.kill(id).await?;
                Ok(())
            }
            Self::Shutdown { network } => {
                let network = network.merge(config.network);
                debug!("Shutting down manager");
                Client::new(network).connect().await?.shutdown().await?;
                Ok(())
            }
        }
    }
}
