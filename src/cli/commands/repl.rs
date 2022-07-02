use crate::{
    cli::{client::ResponseOut, Client},
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

        let mut client = Client::new(config.network).connect().await?;
        let mut channel = client.open_channel(1).await?;
        todo!();
    }
}
