#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistantManagerServerConfig {
    /// Scheme to use when none is provided in a destination
    pub fallback_scheme: String,

    /// Buffer size for queue of incoming connections before blocking
    pub connection_buffer_size: usize,
}

impl Default for DistantManagerServerConfig {
    fn default() -> Self {
        Self {
            fallback_scheme: "distant".to_string(),
            connection_buffer_size: 100,
        }
    }
}
