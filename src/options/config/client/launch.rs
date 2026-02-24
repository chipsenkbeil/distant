use distant_core::net::common::Map;
use serde::{Deserialize, Serialize};

use super::common::BindAddress;

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

#[cfg(test)]
mod tests {
    //! Tests for `ClientLaunchConfig` and `ClientLaunchDistantConfig`: defaults,
    //! `From<Map>` with `distant.*` key extraction, `Into<Map>`, round-trips,
    //! serde, and bind_server parsing for various address types.

    use std::net::Ipv4Addr;

    use distant_core::net::common::Host;
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_has_no_values() {
        let config = ClientLaunchConfig::default();
        assert!(config.distant.bin.is_none());
        assert!(config.distant.bind_server.is_none());
        assert!(config.distant.args.is_none());
        assert!(config.options.is_empty());
    }

    // -------------------------------------------------------
    // From<Map> — empty map
    // -------------------------------------------------------
    #[test]
    fn from_empty_map_produces_default() {
        let config = ClientLaunchConfig::from(Map::new());
        assert_eq!(config, ClientLaunchConfig::default());
    }

    // -------------------------------------------------------
    // From<Map> — fully populated
    // -------------------------------------------------------
    #[test]
    fn from_map_with_all_distant_fields() {
        let mut map = Map::new();
        map.insert("distant.bin".to_string(), "/usr/bin/distant".to_string());
        map.insert("distant.bind_server".to_string(), "any".to_string());
        map.insert("distant.args".to_string(), "--port 8080".to_string());

        let config = ClientLaunchConfig::from(map);
        assert_eq!(config.distant.bin.as_deref(), Some("/usr/bin/distant"));
        assert_eq!(config.distant.bind_server, Some(BindAddress::Any));
        assert_eq!(config.distant.args.as_deref(), Some("--port 8080"));
        assert!(config.options.is_empty());
    }

    // -------------------------------------------------------
    // From<Map> — extra keys go into options
    // -------------------------------------------------------
    #[test]
    fn from_map_extra_keys_become_options() {
        let mut map = Map::new();
        map.insert("distant.bin".to_string(), "distant".to_string());
        map.insert("custom_key".to_string(), "custom_value".to_string());
        map.insert("another".to_string(), "thing".to_string());

        let config = ClientLaunchConfig::from(map);
        assert_eq!(config.distant.bin.as_deref(), Some("distant"));
        assert_eq!(config.options.get("custom_key").unwrap(), "custom_value");
        assert_eq!(config.options.get("another").unwrap(), "thing");
        // distant.bin should NOT be in options — it was consumed
        assert!(config.options.get("distant.bin").is_none());
    }

    // -------------------------------------------------------
    // From<Map> — IP address bind_server parses correctly
    // -------------------------------------------------------
    #[test]
    fn from_map_with_ip_bind_server() {
        let mut map = Map::new();
        // BindAddress::from_str parses "127.0.0.1" as Host::Ipv4, not as an error.
        // Most strings parse successfully (unrecognized ones become Host::Name).
        map.insert("distant.bind_server".to_string(), "127.0.0.1".to_string());

        let config = ClientLaunchConfig::from(map);
        assert_eq!(
            config.distant.bind_server,
            Some(BindAddress::Host(Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1))))
        );
    }

    // -------------------------------------------------------
    // From<Map> — ssh bind_server
    // -------------------------------------------------------
    #[test]
    fn from_map_with_ssh_bind_server() {
        let mut map = Map::new();
        map.insert("distant.bind_server".to_string(), "ssh".to_string());

        let config = ClientLaunchConfig::from(map);
        assert_eq!(config.distant.bind_server, Some(BindAddress::Ssh));
    }

    // -------------------------------------------------------
    // ClientLaunchConfig -> Map (all fields set)
    // -------------------------------------------------------
    #[test]
    fn into_map_with_all_fields() {
        let config = ClientLaunchConfig {
            distant: ClientLaunchDistantConfig {
                bin: Some("my-distant".to_string()),
                bind_server: Some(BindAddress::Any),
                args: Some("--verbose".to_string()),
            },
            options: Map::new(),
        };

        let map: Map = config.into();
        assert_eq!(map.get("distant.bin").unwrap(), "my-distant");
        assert_eq!(map.get("distant.bind_server").unwrap(), "any");
        assert_eq!(map.get("distant.args").unwrap(), "--verbose");
    }

    // -------------------------------------------------------
    // ClientLaunchConfig -> Map (optional fields absent)
    // -------------------------------------------------------
    #[test]
    fn into_map_with_no_optional_fields() {
        let config = ClientLaunchConfig::default();
        let map: Map = config.into();

        assert!(map.get("distant.bin").is_none());
        assert!(map.get("distant.bind_server").is_none());
        assert!(map.get("distant.args").is_none());
    }

    // -------------------------------------------------------
    // ClientLaunchConfig -> Map preserves extra options
    // -------------------------------------------------------
    #[test]
    fn into_map_preserves_options() {
        let mut options = Map::new();
        options.insert("custom".to_string(), "value".to_string());

        let config = ClientLaunchConfig {
            distant: ClientLaunchDistantConfig::default(),
            options,
        };

        let map: Map = config.into();
        assert_eq!(map.get("custom").unwrap(), "value");
    }

    // -------------------------------------------------------
    // Round-trip: Config -> Map -> Config
    // -------------------------------------------------------
    #[test]
    fn round_trip_with_all_fields() {
        let original = ClientLaunchConfig {
            distant: ClientLaunchDistantConfig {
                bin: Some("distant".to_string()),
                bind_server: Some(BindAddress::Any),
                args: Some("--flag".to_string()),
            },
            options: Map::new(),
        };

        let map: Map = original.clone().into();
        let restored = ClientLaunchConfig::from(map);
        assert_eq!(original, restored);
    }

    #[test]
    fn round_trip_default() {
        let original = ClientLaunchConfig::default();
        let map: Map = original.clone().into();
        let restored = ClientLaunchConfig::from(map);
        assert_eq!(original, restored);
    }

    // -------------------------------------------------------
    // Serde round-trip
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip() {
        let config = ClientLaunchConfig {
            distant: ClientLaunchDistantConfig {
                bin: Some("distant".to_string()),
                bind_server: Some(BindAddress::Ssh),
                args: Some("--arg".to_string()),
            },
            options: Map::new(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: ClientLaunchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }

    // -------------------------------------------------------
    // ClientLaunchDistantConfig default
    // -------------------------------------------------------
    #[test]
    fn distant_config_default_has_no_values() {
        let config = ClientLaunchDistantConfig::default();
        assert!(config.bin.is_none());
        assert!(config.bind_server.is_none());
        assert!(config.args.is_none());
    }
}
