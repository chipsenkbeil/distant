use crate::Config;
use clap::Args;
use distant_core::DistantRequestData;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(subcommand)]
    request: DistantRequestData,
}

impl Subcommand {
    pub async fn run(self, config: Config) -> io::Result<()> {
        todo!();
    }
}
