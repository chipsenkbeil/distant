use crate::{
    config::{ClientConfig, ClientShellConfig},
    Merge,
};
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {
    #[clap(flatten)]
    pub config: ClientShellConfig,

    /// Optional command to run instead of $SHELL
    pub cmd: Option<String>,
}

impl Subcommand {
    pub async fn run(self, mut config: ClientConfig) -> io::Result<()> {
        config.merge(self.config);
        todo!();
    }
}
