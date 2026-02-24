use std::path::PathBuf;

use distant_core::net::common::{Map, PortRange};
use distant_core::net::server::Shutdown;
use serde::{Deserialize, Serialize};

use crate::options::BindAddress;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerListenConfig {
    pub host: Option<BindAddress>,
    pub port: Option<PortRange>,
    pub use_ipv6: bool,
    pub shutdown: Option<Shutdown>,
    pub current_dir: Option<PathBuf>,
}

impl From<Map> for ServerListenConfig {
    fn from(mut map: Map) -> Self {
        Self {
            host: map
                .remove("host")
                .and_then(|x| x.parse::<BindAddress>().ok()),
            port: map.remove("port").and_then(|x| x.parse::<PortRange>().ok()),
            use_ipv6: map
                .remove("use_ipv6")
                .and_then(|x| x.parse::<bool>().ok())
                .unwrap_or_default(),
            shutdown: map
                .remove("shutdown")
                .and_then(|x| x.parse::<Shutdown>().ok()),
            current_dir: map
                .remove("current_dir")
                .and_then(|x| x.parse::<PathBuf>().ok()),
        }
    }
}

impl From<ServerListenConfig> for Map {
    fn from(config: ServerListenConfig) -> Self {
        let mut this = Self::new();

        if let Some(x) = config.host {
            this.insert("host".to_string(), x.to_string());
        }

        if let Some(x) = config.port {
            this.insert("port".to_string(), x.to_string());
        }

        this.insert("use_ipv6".to_string(), config.use_ipv6.to_string());

        if let Some(x) = config.shutdown {
            this.insert("shutdown".to_string(), x.to_string());
        }

        if let Some(x) = config.current_dir {
            this.insert("current_dir".to_string(), x.to_string_lossy().to_string());
        }

        this
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `ServerListenConfig`: defaults, `From<Map>` with valid/invalid
    //! inputs (port, use_ipv6, shutdown variants, port ranges), `Into<Map>`,
    //! round-trips, and serde.

    use std::net::Ipv4Addr;
    use std::time::Duration;

    use distant_core::net::common::{Host, PortRange};
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_has_no_values_except_use_ipv6_false() {
        let config = ServerListenConfig::default();
        assert!(config.host.is_none());
        assert!(config.port.is_none());
        assert!(!config.use_ipv6);
        assert!(config.shutdown.is_none());
        assert!(config.current_dir.is_none());
    }

    // -------------------------------------------------------
    // From<Map> — empty map
    // -------------------------------------------------------
    #[test]
    fn from_empty_map_produces_default() {
        let config = ServerListenConfig::from(Map::new());
        assert_eq!(config, ServerListenConfig::default());
    }

    // -------------------------------------------------------
    // From<Map> — fully populated
    // -------------------------------------------------------
    #[test]
    fn from_map_with_all_fields() {
        let mut map = Map::new();
        map.insert("host".to_string(), "127.0.0.1".to_string());
        map.insert("port".to_string(), "8080".to_string());
        map.insert("use_ipv6".to_string(), "true".to_string());
        map.insert("shutdown".to_string(), "after=60".to_string());
        map.insert("current_dir".to_string(), "/tmp/test".to_string());

        let config = ServerListenConfig::from(map);

        assert_eq!(
            config.host,
            Some(BindAddress::Host(Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1))))
        );
        assert_eq!(
            config.port,
            Some(PortRange {
                start: 8080,
                end: None
            })
        );
        assert!(config.use_ipv6);
        assert_eq!(
            config.shutdown,
            Some(Shutdown::After(Duration::from_secs(60)))
        );
        assert_eq!(config.current_dir, Some(PathBuf::from("/tmp/test")));
    }

    // -------------------------------------------------------
    // From<Map> — partial fields
    // -------------------------------------------------------
    #[test]
    fn from_map_with_host_only() {
        let mut map = Map::new();
        map.insert("host".to_string(), "any".to_string());

        let config = ServerListenConfig::from(map);
        assert_eq!(config.host, Some(BindAddress::Any));
        assert!(config.port.is_none());
        assert!(!config.use_ipv6);
        assert!(config.shutdown.is_none());
        assert!(config.current_dir.is_none());
    }

    // -------------------------------------------------------
    // From<Map> — invalid values are skipped (parsed as None)
    // -------------------------------------------------------
    #[test]
    fn from_map_with_invalid_port_skips_it() {
        let mut map = Map::new();
        map.insert("port".to_string(), "not-a-port".to_string());

        let config = ServerListenConfig::from(map);
        assert!(config.port.is_none());
    }

    #[test]
    fn from_map_with_invalid_use_ipv6_defaults_to_false() {
        let mut map = Map::new();
        map.insert("use_ipv6".to_string(), "not-a-bool".to_string());

        let config = ServerListenConfig::from(map);
        assert!(!config.use_ipv6);
    }

    #[test]
    fn from_map_with_invalid_shutdown_skips_it() {
        let mut map = Map::new();
        map.insert("shutdown".to_string(), "invalid".to_string());

        let config = ServerListenConfig::from(map);
        assert!(config.shutdown.is_none());
    }

    // -------------------------------------------------------
    // From<Map> — port range
    // -------------------------------------------------------
    #[test]
    fn from_map_with_port_range() {
        let mut map = Map::new();
        map.insert("port".to_string(), "8080:8090".to_string());

        let config = ServerListenConfig::from(map);
        assert_eq!(
            config.port,
            Some(PortRange {
                start: 8080,
                end: Some(8090)
            })
        );
    }

    // -------------------------------------------------------
    // From<Map> — shutdown variants
    // -------------------------------------------------------
    #[test]
    fn from_map_with_shutdown_never() {
        let mut map = Map::new();
        map.insert("shutdown".to_string(), "never".to_string());

        let config = ServerListenConfig::from(map);
        assert_eq!(config.shutdown, Some(Shutdown::Never));
    }

    #[test]
    fn from_map_with_shutdown_lonely() {
        let mut map = Map::new();
        map.insert("shutdown".to_string(), "lonely=30".to_string());

        let config = ServerListenConfig::from(map);
        assert_eq!(
            config.shutdown,
            Some(Shutdown::Lonely(Duration::from_secs(30)))
        );
    }

    // -------------------------------------------------------
    // ServerListenConfig -> Map (all fields set)
    // -------------------------------------------------------
    #[test]
    fn into_map_with_all_fields() {
        let config = ServerListenConfig {
            host: Some(BindAddress::Host(Host::Ipv4(Ipv4Addr::new(10, 0, 0, 1)))),
            port: Some(PortRange {
                start: 3000,
                end: None,
            }),
            use_ipv6: true,
            shutdown: Some(Shutdown::Never),
            current_dir: Some(PathBuf::from("/home/user")),
        };

        let map: Map = config.into();
        assert_eq!(map.get("host").unwrap(), "10.0.0.1");
        assert_eq!(map.get("port").unwrap(), "3000");
        assert_eq!(map.get("use_ipv6").unwrap(), "true");
        assert_eq!(map.get("shutdown").unwrap(), "never");
        assert_eq!(map.get("current_dir").unwrap(), "/home/user");
    }

    // -------------------------------------------------------
    // ServerListenConfig -> Map (optional fields absent)
    // -------------------------------------------------------
    #[test]
    fn into_map_with_no_optional_fields() {
        let config = ServerListenConfig::default();
        let map: Map = config.into();

        assert!(map.get("host").is_none());
        assert!(map.get("port").is_none());
        assert_eq!(map.get("use_ipv6").unwrap(), "false");
        assert!(map.get("shutdown").is_none());
        assert!(map.get("current_dir").is_none());
    }

    // -------------------------------------------------------
    // Round-trip: Config -> Map -> Config
    // -------------------------------------------------------
    #[test]
    fn round_trip_all_fields() {
        let original = ServerListenConfig {
            host: Some(BindAddress::Any),
            port: Some(PortRange {
                start: 5000,
                end: Some(5010),
            }),
            use_ipv6: false,
            shutdown: Some(Shutdown::After(Duration::from_secs(120))),
            current_dir: Some(PathBuf::from("/var/data")),
        };

        let map: Map = original.clone().into();
        let restored = ServerListenConfig::from(map);
        assert_eq!(original, restored);
    }

    #[test]
    fn round_trip_default() {
        let original = ServerListenConfig::default();
        let map: Map = original.clone().into();
        let restored = ServerListenConfig::from(map);
        assert_eq!(original, restored);
    }

    // -------------------------------------------------------
    // From<Map> — host variants
    // -------------------------------------------------------
    #[test]
    fn from_map_with_ssh_host() {
        let mut map = Map::new();
        map.insert("host".to_string(), "ssh".to_string());

        let config = ServerListenConfig::from(map);
        assert_eq!(config.host, Some(BindAddress::Ssh));
    }

    // -------------------------------------------------------
    // Serde round-trip
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip() {
        let config = ServerListenConfig {
            host: Some(BindAddress::Any),
            port: Some(0.into()),
            use_ipv6: true,
            shutdown: Some(Shutdown::Never),
            current_dir: Some(PathBuf::from("/tmp")),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: ServerListenConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }
}
