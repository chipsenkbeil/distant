use super::{CommonConfig, NetworkConfig};
use crate::Merge;
use serde::{Deserialize, Serialize};

mod launch;
pub use launch::*;

mod lsp;
pub use lsp::*;

mod repl;
pub use repl::*;

mod shell;
pub use shell::*;

/// Represents configuration settings for the distant client
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(flatten)]
    pub common: CommonConfig,

    pub launch: ClientLaunchConfig,
    pub lsp: ClientLspConfig,
    pub repl: ClientReplConfig,
    pub shell: ClientShellConfig,

    #[serde(flatten)]
    pub network: NetworkConfig,
}

impl Merge for ClientConfig {
    fn merge(&mut self, other: Self) {
        self.common.merge(other.common);
        self.launch.merge(other.launch);
        self.lsp.merge(other.lsp);
        self.shell.merge(other.shell);
        self.network.merge(other.network);
    }
}

impl Merge<CommonConfig> for ClientConfig {
    fn merge(&mut self, other: CommonConfig) {
        self.common.merge(other);
    }
}

impl Merge<ClientLaunchConfig> for ClientConfig {
    fn merge(&mut self, other: ClientLaunchConfig) {
        self.launch.merge(other);
    }
}

impl Merge<ClientLspConfig> for ClientConfig {
    fn merge(&mut self, other: ClientLspConfig) {
        self.lsp.merge(other);
    }
}

impl Merge<ClientReplConfig> for ClientConfig {
    fn merge(&mut self, other: ClientReplConfig) {
        self.repl.merge(other);
    }
}

impl Merge<ClientShellConfig> for ClientConfig {
    fn merge(&mut self, other: ClientShellConfig) {
        self.shell.merge(other);
    }
}

impl Merge<NetworkConfig> for ClientConfig {
    fn merge(&mut self, other: NetworkConfig) {
        self.network.merge(other);
    }
}
