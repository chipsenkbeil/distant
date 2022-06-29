use crate::Merge;
use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientShellConfig {
    /// If provided, will run in persist mode, meaning that the process will not be killed if the
    /// client disconnects from the server
    #[clap(long)]
    pub persist: bool,
}

impl Merge for ClientShellConfig {
    fn merge(&mut self, other: Self) {
        self.persist = other.persist;
    }
}
