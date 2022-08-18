use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Represents a general-purpose set of properties tied with a server instance
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// If provided, will cause server to shut down if duration is exceeded with no active
    /// connections
    pub shutdown_after: Option<Duration>,
}
