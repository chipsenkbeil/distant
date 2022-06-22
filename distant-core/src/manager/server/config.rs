use crate::ConnectHandler;
use std::collections::HashMap;

pub struct DistantManagerConfig {
    /// Scheme to use when none is provided in a destination
    pub fallback_scheme: String,

    /// Buffer size for queue of incoming connections before blocking
    pub connection_buffer_size: usize,

    /// Handlers to use for connect requests
    pub handlers: HashMap<String, Box<dyn ConnectHandler + Send + Sync>>,
}

impl Default for DistantManagerConfig {
    fn default() -> Self {
        Self {
            fallback_scheme: "distant".to_string(),
            connection_buffer_size: 100,
            handlers: HashMap::new(),
        }
    }
}
