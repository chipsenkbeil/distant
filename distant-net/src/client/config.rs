use super::ReconnectStrategy;
use std::time::Duration;

const DEFAULT_SILENCE_DURATION: Duration = Duration::from_secs(20);
const MAXIMUM_SILENCE_DURATION: Duration = Duration::from_millis(68719476734);

/// Represents a general-purpose set of properties tied with a client instance.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Strategy to use when reconnecting to a server.
    pub reconnect_strategy: ReconnectStrategy,

    /// A maximum duration to not receive any response/heartbeat from a server before deeming the
    /// server as lost and triggering a reconnect.
    pub silence_duration: Duration,
}

impl ClientConfig {
    pub fn with_maximum_silence_duration(self) -> Self {
        Self {
            reconnect_strategy: self.reconnect_strategy,
            silence_duration: MAXIMUM_SILENCE_DURATION,
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            reconnect_strategy: ReconnectStrategy::Fail,
            silence_duration: DEFAULT_SILENCE_DURATION,
        }
    }
}
