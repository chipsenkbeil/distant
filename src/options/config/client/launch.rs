use super::common::BindAddress;
use distant_core::net::common::Map;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientLaunchConfig {
    #[serde(flatten)]
    pub distant: ClientLaunchDistantConfig,
    pub options: Map,
}

impl From<Map> for ClientLaunchConfig {
    fn from(mut map: Map) -> Self {
        Self {
            distant: ClientLaunchDistantConfig {
                bin: map.remove("distant.bin"),
                bind_server: map
                    .remove("distant.bind_server")
                    .and_then(|x| x.parse::<BindAddress>().ok()),
                args: map.remove("distant.args"),
            },
            options: map,
        }
    }
}

impl From<ClientLaunchConfig> for Map {
    fn from(config: ClientLaunchConfig) -> Self {
        let mut this = Self::new();

        if let Some(x) = config.distant.bin {
            this.insert("distant.bin".to_string(), x);
        }

        if let Some(x) = config.distant.bind_server {
            this.insert("distant.bind_server".to_string(), x.to_string());
        }

        if let Some(x) = config.distant.args {
            this.insert("distant.args".to_string(), x);
        }

        this.extend(config.options);

        this
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientLaunchDistantConfig {
    pub bin: Option<String>,
    pub bind_server: Option<BindAddress>,
    pub args: Option<String>,
}
