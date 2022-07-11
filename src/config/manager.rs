use super::{CommonConfig, NetworkConfig};
use crate::cli::ServiceKind;
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
