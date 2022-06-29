use crate::{
    cli::{client::ResponseOut, Client},
    config::{ClientConfig, ClientLaunchConfig, NetworkConfig},
    Merge,
};
use clap::Args;
use distant_core::Destination;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ClientLaunchConfig,

    #[clap(flatten)]
    pub network: NetworkConfig,

    #[clap(name = "DESTINATION")]
    pub destination: Destination,
}

impl Subcommand {
    pub async fn run(self, mut config: ClientConfig) -> io::Result<()> {
        config.merge(self.config);
        config.merge(self.network);

        todo!();
    }
}
