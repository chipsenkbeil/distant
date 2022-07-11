use crate::{
    cli::{
        CliResult, Client, Manager, Service, ServiceInstallCtx, ServiceKind, ServiceStartCtx,
        ServiceStopCtx, ServiceUninstallCtx,
    },
    config::ManagerConfig,
};
use clap::Subcommand;
use distant_core::{net::ServerRef, ConnectionId, DistantManagerConfig};
use log::*;
use std::io;
use tabled::{Table, Tabled};

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
    },

    /// Retrieve information about a specific connection
    Info { id: ConnectionId },

    /// List information about all connections
    List,

    /// Kill a specific connection
    Kill { id: ConnectionId },

    /// Send a shutdown request to the manager
    Shutdown,
}

#[derive(Debug, Subcommand)]
pub enum ManagerServiceSubcommand {
    /// Start the manager as a service
    Start {
        /// Type of service manager used to run this service
        #[clap(default_value_t = ServiceKind::default(), value_enum)]
        kind: ServiceKind,
    },

    /// Stop the manager as a service
    Stop {
        /// Type of service manager used to run this service
        #[clap(default_value_t = ServiceKind::default(), value_enum)]
        kind: ServiceKind,
    },

    /// Install the manager as a service
    Install {
        #[clap(default_value_t = ServiceKind::default(), value_enum)]
        kind: ServiceKind,

        /// If specified, installs as a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Uninstall the manager as a service
    Uninstall {
        #[clap(default_value_t = ServiceKind::default(), value_enum)]
        kind: ServiceKind,

        /// If specified, uninstalls a user-level service
        #[clap(long)]
        user: bool,
    },
}

impl ManagerSubcommand {
    pub fn run(self, config: ManagerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon } if *daemon => Self::run_daemon(self, config),
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
                let service = <dyn Service>::target(kind);
                service.start(ServiceStartCtx {
                    label: String::from("rocks.distant.manager"),
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Stop { kind }) => {
                debug!("Stopping manager service via {:?}", kind);
                let service = <dyn Service>::target(kind);
                service.stop(ServiceStopCtx {
                    label: String::from("rocks.distant.manager"),
                })?;
                Ok(())
            }

            Self::Service(ManagerServiceSubcommand::Install { kind, user }) => {
                debug!("Installing manager service via {:?}", kind);
                let service = <dyn Service>::target(kind);
                service.install(ServiceInstallCtx {
                    label: String::from("rocks.distant.manager"),
                    user,

                    // distant manager listen
                    args: vec![
                        std::env::current_exe()
                            .ok()
                            .and_then(|p| p.to_str().map(ToString::to_string))
                            .unwrap_or_else(|| String::from("distant")),
                        "manager".to_string(),
                        "listen".to_string(),
                    ],
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Uninstall { kind, user }) => {
                debug!("Uninstalling manager service via {:?}", kind);
                let service = <dyn Service>::target(kind);
                service.uninstall(ServiceUninstallCtx {
                    label: String::from("rocks.distant.manager"),
                    user,
                })?;
                Ok(())
            }

            Self::Listen { .. } => {
                debug!("Starting manager: {:?}", config.network.as_os_str());
                let manager_ref = Manager::new(DistantManagerConfig::default(), config.network)
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
                debug!("Manager is shutting down");

                Ok(())
            }
            Self::Info { id } => {
                debug!(
                    "Getting info about connection {} from manager: {:?}",
                    id,
                    config.network.as_os_str()
                );
                let info = Client::new(config.network)
                    .connect()
                    .await?
                    .info(id)
                    .await?;

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
            Self::List => {
                debug!(
                    "Getting list of connections from manager: {:?}",
                    config.network.as_os_str()
                );
                let list = Client::new(config.network).connect().await?.list().await?;

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
            Self::Kill { id } => {
                debug!(
                    "Killing connection {} from manager: {:?}",
                    id,
                    config.network.as_os_str()
                );
                Client::new(config.network)
                    .connect()
                    .await?
                    .kill(id)
                    .await?;
                Ok(())
            }
            Self::Shutdown => {
                debug!("Shutting down manager: {:?}", config.network.as_os_str());
                Client::new(config.network)
                    .connect()
                    .await?
                    .shutdown()
                    .await?;
                Ok(())
            }
        }
    }
}
