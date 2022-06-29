use crate::{
    config::{ClientConfig, ClientReplConfig},
    Merge,
};
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ClientReplConfig,
}

impl Subcommand {
    pub async fn run(self, mut config: ClientConfig) -> io::Result<()> {
        config.merge(self.config);
        todo!();
    }
}
