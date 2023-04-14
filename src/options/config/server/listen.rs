use anyhow::Context;
use distant_core::net::common::{Host, HostParseError, Map, PortRange};
use distant_core::net::server::Shutdown;
use serde::{Deserialize, Serialize};
use std::{
    env, fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    str::FromStr,
};

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
