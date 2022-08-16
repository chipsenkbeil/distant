use crate::{BoxedConnectHandler, BoxedLaunchHandler};
use std::collections::HashMap;

pub struct DistantManagerConfig {
    /// Scheme to use when none is provided in a destination for launch
    pub launch_fallback_scheme: String,

    /// Scheme to use when none is provided in a destination for connect
    pub connect_fallback_scheme: String,

    /// Buffer size for queue of incoming connections before blocking
    pub connection_buffer_size: usize,

    /// If listening as local user
    pub user: bool,

    /// Handlers to use for launch requests
    pub launch_handlers: HashMap<String, BoxedLaunchHandler>,

    /// Handlers to use for connect requests
    pub connect_handlers: HashMap<String, BoxedConnectHandler>,
}

impl Default for DistantManagerConfig {
    fn default() -> Self {
        Self {
            // Default to using ssh to launch distant
            launch_fallback_scheme: "ssh".to_string(),

            // Default to distant server when connecting
            connect_fallback_scheme: "distant".to_string(),

            connection_buffer_size: 100,
            user: false,
            launch_handlers: HashMap::new(),
            connect_handlers: HashMap::new(),
        }
    }
}
