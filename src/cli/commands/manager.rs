use crate::{
    cli::{CliResult, Client},
    config::{ManagerConfig, ServiceKind},
    Merge,
};
use clap::Subcommand;
use distant_core::DistantManager;

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
    Info { id: usize },

    /// List information about all connections
    List,

    /// Kill a specific connection
    Kill { id: usize },
}

impl ManagerSubcommand {
    pub async fn run(self, config: ManagerConfig) -> CliResult<()> {
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

            Self::Listen { daemon } => todo!(),
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
