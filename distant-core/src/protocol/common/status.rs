use serde::{Deserialize, Serialize};

use super::TunnelInfo;

/// Aggregated status information from the server.
///
/// Currently contains active tunnel state; future versions may include
/// watcher and process information as backends add support.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusInfo {
    /// Active tunnels and listeners.
    pub tunnels: Vec<TunnelInfo>,
}
