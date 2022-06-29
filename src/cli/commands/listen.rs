use crate::{
    config::{ServerConfig, ServerListenConfig},
    Merge,
};
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ServerListenConfig,
}

impl Subcommand {
    pub async fn run(self, mut config: ServerConfig) -> io::Result<()> {
        config.merge(self.config);
        todo!();
    }
}
