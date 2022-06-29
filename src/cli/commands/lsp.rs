use crate::{
    config::{ClientConfig, ClientLspConfig, NetworkConfig},
    Merge,
};
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ClientLspConfig,

    #[clap(flatten)]
    pub network: NetworkConfig,

    pub cmd: String,
}

impl Subcommand {
    pub async fn run(self, mut config: ClientConfig) -> io::Result<()> {
        config.merge(self.config);
        config.merge(self.network);
        todo!();
    }
}
