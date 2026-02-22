use std::collections::HashMap;
use std::sync::Arc;

use crate::plugin::Plugin;

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

    /// Plugins keyed by scheme. Each scheme maps to a plugin that handles
    /// both launch and connect for that scheme.
    pub plugins: HashMap<String, Arc<dyn Plugin>>,
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
        }
    }
}
