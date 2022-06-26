use distant_core::Destination;
use merge::Merge;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant manager
#[derive(Debug, Default, Merge, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[merge(strategy = merge::vec::append)]
    pub connections: Vec<ManagerConnectionConfig>,
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
