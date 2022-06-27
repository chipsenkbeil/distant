use crate::Config;
use clap::Args;
use std::io;

#[derive(Args, Debug)]
pub struct Subcommand {}

impl Subcommand {
    pub async fn run(self, config: Config) -> io::Result<()> {
        todo!();
    }
}
