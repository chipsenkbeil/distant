use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::plugin::{MountPlugin, Plugin};

/// Default polling interval for the per-mount health monitor.
const DEFAULT_MOUNT_HEALTH_INTERVAL: Duration = Duration::from_secs(5);

/// Configuration settings for a manager.
pub struct Config {
    /// Scheme to use when none is provided in a destination for launch
    pub launch_fallback_scheme: String,

    /// Scheme to use when none is provided in a destination for connect
    pub connect_fallback_scheme: String,

    /// Buffer size for queue of incoming connections before blocking
    pub connection_buffer_size: usize,

    /// If listening as local user
    pub user: bool,

    /// Connection plugins keyed by scheme. Each scheme maps to a plugin that
    /// handles both launch and connect for that scheme.
    pub plugins: HashMap<String, Arc<dyn Plugin>>,

    /// Mount plugins keyed by backend name (e.g., "nfs", "fuse").
    pub mount_plugins: HashMap<String, Arc<dyn MountPlugin>>,

    /// How often the per-mount monitor task polls each mount handle
    /// for [`MountProbe`](crate::plugin::MountProbe) liveness signals.
    /// Defaults to 5 seconds.
    pub mount_health_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Default to using ssh to launch distant
            launch_fallback_scheme: "ssh".to_string(),

            // Default to distant server when connecting
            connect_fallback_scheme: "distant".to_string(),

            connection_buffer_size: 100,
            user: false,
            plugins: HashMap::new(),
            mount_plugins: HashMap::new(),
            mount_health_interval: DEFAULT_MOUNT_HEALTH_INTERVAL,
        }
    }
}
