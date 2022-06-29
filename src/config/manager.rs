use super::{CommonConfig, NetworkConfig, ServiceKind};
use crate::Merge;
use clap::Args;
use distant_core::Destination;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant manager
#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[clap(flatten)]
    #[serde(flatten)]
    pub common: CommonConfig,

    #[clap(skip)]
    pub connections: Vec<ManagerConnectionConfig>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub network: NetworkConfig,

    #[clap(value_enum)]
    pub service: Option<ServiceKind>,
}

impl Merge for ManagerConfig {
    fn merge(&mut self, other: Self) {
        self.common.merge(other.common);
        self.connections.extend(other.connections);
        self.network.merge(other.network);
        if let Some(x) = other.service {
            self.service = Some(x);
        }
    }
}

impl Merge<CommonConfig> for ManagerConfig {
    fn merge(&mut self, other: CommonConfig) {
        self.common.merge(other);
    }
}

impl Merge<NetworkConfig> for ManagerConfig {
    fn merge(&mut self, other: NetworkConfig) {
        self.network.merge(other);
    }
}

impl Merge<ServiceKind> for ManagerConfig {
    fn merge(&mut self, other: ServiceKind) {
        self.service = Some(other);
    }
}

/// Represents configuration for some managed connection
#[derive(Debug, Serialize, Deserialize)]
pub enum ManagerConnectionConfig {
    Distant(ManagerDistantConnectionConfig),
    Ssh(ManagerSshConnectionConfig),
}

/// Represents configuration for a distant connection
#[derive(Debug, Serialize, Deserialize)]
pub struct ManagerDistantConnectionConfig {
    pub name: String,
    pub destination: Destination,
    pub key_cmd: Option<String>,
}

/// Represents configuration for an SSH connection
#[derive(Debug, Serialize, Deserialize)]
pub struct ManagerSshConnectionConfig {
    pub name: String,
    pub destination: Destination,
}
