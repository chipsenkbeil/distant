use crate::Merge;
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ReplFormat {
    /// Sends and receives data in JSON format
    Json,

    /// Commands are traditional shell commands and output responses are
    /// inline with what is expected of a program's output in a shell
    Shell,
}

impl Default for ReplFormat {
    fn default() -> Self {
        Self::Shell
    }
}

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientReplConfig {
    /// Represents the format that is used to communicate using the repl
    ///
    /// Currently, there are two possible formats:
    ///
    /// 1. "json": printing out JSON for external program usage
    ///
    /// 2. "shell": clapprinting out human-readable results for interactive shell usage
    #[clap(short, long, value_enum)]
    pub format: Option<ReplFormat>,
}

impl Merge for ClientReplConfig {
    fn merge(&mut self, other: Self) {
        if let Some(x) = other.format {
            self.format = Some(x);
        }
    }
}
