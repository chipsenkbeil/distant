use distant_core::net::common::Map;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConnectConfig {
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
