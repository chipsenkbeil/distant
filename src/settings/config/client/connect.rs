use clap::Args;
use distant_core::net::common::Map;
use serde::{Deserialize, Serialize};

#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConnectConfig {
    /// Additional options to provide, typically forwarded to the handler within the manager
    /// facilitating the connection. Options are key-value pairs separated by comma.
    ///
    /// E.g. `key="value",key2="value2"`
    #[clap(long, default_value_t)]
    pub options: Map,
}

impl From<Map> for ClientConnectConfig {
    fn from(map: Map) -> Self {
        Self { options: map }
    }
}

impl From<ClientConnectConfig> for Map {
    fn from(config: ClientConnectConfig) -> Self {
        let mut this = Self::new();
        this.extend(config.options);
        this
    }
}
