use crate::{
    cli::{client::ResponseOut, Client},
    config::{ClientConfig, NetworkConfig, ReplFormat},
    Merge,
};
use clap::Args;
use distant_core::{DistantMsg, DistantRequestData};
use std::{io, time::Duration};

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub network: NetworkConfig,

    #[clap(subcommand)]
    pub request: DistantRequestData,
}

impl Subcommand {
    pub async fn run(self, mut config: ClientConfig) -> io::Result<()> {
        config.merge(self.network);

        let mut client = Client::new(config.network).connect().await?;
        let mut channel = client.open_channel(1).await?;
        let response = match config.common.timeout {
            Some(secs) => {
                channel
                    .send_timeout(
                        DistantMsg::Single(self.request),
                        Duration::from_secs_f32(secs),
                    )
                    .await?
            }
            None => channel.send(DistantMsg::Single(self.request)).await?,
        };

        ResponseOut::new(ReplFormat::Shell, response)?.print();

        Ok(())
    }
}