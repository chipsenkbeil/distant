use crate::{
    config::{ManagerConfig, ServiceKind},
    Merge,
};
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ManagerConfig,

    #[clap(subcommand)]
    pub cmd: ManagerSubcommand,
}

impl Subcommand {
    pub async fn run(self, mut config: ManagerConfig) -> io::Result<()> {
        config.merge(self.config);
        self.cmd.run(config).await
    }
}

#[derive(Debug, clap::Subcommand)]
pub enum ManagerSubcommand {
    /// Start the manager
    Start {
        /// Type of service manager used to run this service
        #[clap(value_enum)]
        kind: ServiceKind,
    },

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

    /// Retrieve information about a specific connection
    Info { id: usize },

    /// List information about all connections
    List,

    /// Kill a specific connection
    Kill { id: usize },

    /// Stop the manager
    Stop,
}

impl ManagerSubcommand {
    pub async fn run(self, config: ManagerConfig) -> io::Result<()> {
        match self {
            Self::Start { kind } => todo!(),
            Self::Install { kind } => todo!(),
            Self::Uninstall { kind } => todo!(),
            Self::Info { id } => todo!(),
            Self::List => todo!(),
            Self::Kill { id } => todo!(),
            Self::Stop => todo!(),
        }
    }
}
