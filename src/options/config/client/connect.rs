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

#[cfg(test)]
mod tests {
    //! Tests for `ClientConnectConfig`: defaults, `From<Map>`, `Into<Map>`,
    //! round-trips, and serde serialization.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_has_empty_options() {
        let config = ClientConnectConfig::default();
        assert!(config.options.is_empty());
    }

    // -------------------------------------------------------
    // From<Map> — empty
    // -------------------------------------------------------
    #[test]
    fn from_empty_map_produces_default() {
        let config = ClientConnectConfig::from(Map::new());
        assert_eq!(config, ClientConnectConfig::default());
    }

    // -------------------------------------------------------
    // From<Map> — with data
    // -------------------------------------------------------
    #[test]
    fn from_map_with_data() {
        let mut map = Map::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());

        let config = ClientConnectConfig::from(map);
        assert_eq!(config.options.get("key1").unwrap(), "value1");
        assert_eq!(config.options.get("key2").unwrap(), "value2");
    }

    // -------------------------------------------------------
    // ClientConnectConfig -> Map
    // -------------------------------------------------------
    #[test]
    fn into_map_preserves_options() {
        let mut options = Map::new();
        options.insert("key".to_string(), "value".to_string());
        let config = ClientConnectConfig { options };

        let map: Map = config.into();
        assert_eq!(map.get("key").unwrap(), "value");
    }

    // -------------------------------------------------------
    // Round-trip
    // -------------------------------------------------------
    #[test]
    fn round_trip() {
        let mut options = Map::new();
        options.insert("a".to_string(), "b".to_string());
        let original = ClientConnectConfig { options };

        let map: Map = original.clone().into();
        let restored = ClientConnectConfig::from(map);
        assert_eq!(original, restored);
    }

    #[test]
    fn round_trip_empty() {
        let original = ClientConnectConfig::default();
        let map: Map = original.clone().into();
        let restored = ClientConnectConfig::from(map);
        assert_eq!(original, restored);
    }

    // -------------------------------------------------------
    // Serde
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip() {
        let mut options = Map::new();
        options.insert("test_key".to_string(), "test_value".to_string());
        let config = ClientConnectConfig { options };

        let json = serde_json::to_string(&config).unwrap();
        let restored: ClientConnectConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }
}
