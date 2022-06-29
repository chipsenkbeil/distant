use crate::Merge;
use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientLspConfig {
    /// If provided, will run in persist mode, meaning that the process will not be killed if the
    /// client disconnects from the server
    #[clap(long)]
    pub persist: bool,

    /// If provided, will run LSP in a pty
    #[clap(long)]
    pub pty: bool,
}

impl Merge for ClientLspConfig {
    fn merge(&mut self, other: Self) {
        self.persist = other.persist;
        self.pty = other.pty;
    }
}
