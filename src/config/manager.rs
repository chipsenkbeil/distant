use super::{AccessControl, CommonConfig, NetworkConfig};
use clap::Args;
use distant_core::net::common::Destination;
use serde::{Deserialize, Serialize};
use service_manager::ServiceManagerKind;

/// Represents configuration settings for the distant manager
#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ManagerConfig {
    /// Type of access to apply to created unix socket or windows pipe
    #[clap(long, value_enum)]
    pub access: Option<AccessControl>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub common: CommonConfig,

    #[clap(skip)]
    pub connections: Vec<ManagerConnectionConfig>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub network: NetworkConfig,

    #[clap(value_enum)]
    pub service: Option<ServiceManagerKind>,
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
