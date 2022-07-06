use crate::{
    cli::{CliResult, Client, Manager},
    config::{ManagerConfig, ServiceKind},
};
use clap::Subcommand;
use distant_core::{net::ServerRef, ConnectionId, DistantManagerConfig};

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
            Self::Start { kind } => todo!(),
            Self::Stop => {
                let _ = Client::new(config.network)
                    .connect()
                    .await?
                    .shutdown()
                    .await?;
                Ok(())
            }

            Self::Install { kind } => todo!(),
            Self::Uninstall { kind } => todo!(),

            Self::Listen { daemon } => {
                let manager_ref = Manager::new(DistantManagerConfig::default(), config.network)
                    .listen()
                    .await?;

                // Register our handlers for different schemes
                manager_ref
                    .register_launch_handler("local", handlers::LocalLaunchHandler)
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

                Ok(())
            }
            Self::Info { id } => {
                let info = Client::new(config.network)
                    .connect()
                    .await?
                    .info(id)
                    .await?;

                println!("id: {}", info.id);
                println!("destination: {}", info.destination);
                println!("extra: {}", info.extra);

                Ok(())
            }
            Self::List => {
                let list = Client::new(config.network).connect().await?.list().await?;

                for (id, destination) in list {
                    println!("{}: {}", id, destination);
                }

                Ok(())
            }
            Self::Kill { id } => {
                let _ = Client::new(config.network)
                    .connect()
                    .await?
                    .kill(id)
                    .await?;
                Ok(())
            }
        }
    }
}
