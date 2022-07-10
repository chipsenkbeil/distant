use crate::{
    cli::{CliResult, Client, Manager},
    config::{ManagerConfig, ServiceKind},
};
use clap::Subcommand;
use distant_core::{net::ServerRef, ConnectionId, DistantManagerConfig};
use log::*;
use tabled::{Table, Tabled};

mod handlers;

#[derive(Debug, Subcommand)]
pub enum ManagerSubcommand {
    /// Start the manager as a service
    Start {
        /// Type of service manager used to run this service
        #[clap(value_enum)]
        kind: ServiceKind,
    },

    /// Stop the manager as a service
    Stop,

    /// Install the manager as a service
    Install {
        #[clap(value_enum)]
        kind: ServiceKind,
    },

    /// Uninstall the manager as a service
    Uninstall {
        #[clap(value_enum)]
        kind: ServiceKind,
    },

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
}

impl ManagerSubcommand {
    pub fn run(self, config: ManagerConfig) -> CliResult<()> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(Self::async_run(self, config))
    }

    async fn async_run(self, config: ManagerConfig) -> CliResult<()> {
        match self {
            Self::Start { .. } => todo!(),
            Self::Stop => {
                debug!("Stopping manager: {:?}", config.network.as_os_str());
                Client::new(config.network)
                    .connect()
                    .await?
                    .shutdown()
                    .await?;
                Ok(())
            }

            Self::Install { .. } => todo!(),
            Self::Uninstall { .. } => todo!(),

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
        }
    }
}
