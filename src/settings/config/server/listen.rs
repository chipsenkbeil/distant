use anyhow::Context;
use clap::Args;
use distant_core::net::common::{Host, HostParseError, Map, PortRange};
use distant_core::net::server::Shutdown;
use serde::{Deserialize, Serialize};
use std::{
    env, fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    str::FromStr,
};

#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerListenConfig {
    /// Control the IP address that the distant binds to
    ///
    /// There are three options here:
    ///
    /// 1. `ssh`: the server will reply from the IP address that the SSH
    /// connection came from (as found in the SSH_CONNECTION environment variable). This is
    /// useful for multihomed servers.
    ///
    /// 2. `any`: the server will reply on the default interface and will not bind to
    /// a particular IP address. This can be useful if the connection is made through ssh or
    /// another tool that makes the SSH connection appear to come from localhost.
    ///
    /// 3. `IP`: the server will attempt to bind to the specified IP address.
    #[clap(long, value_name = "ssh|any|IP")]
    pub host: Option<BindAddress>,

    /// Set the port(s) that the server will attempt to bind to
    ///
    /// This can be in the form of PORT1 or PORT1:PORTN to provide a range of ports.
    /// With `--port 0`, the server will let the operating system pick an available TCP port.
    ///
    /// Please note that this option does not affect the server-side port used by SSH
    #[clap(long, value_name = "PORT[:PORT2]")]
    pub port: Option<PortRange>,

    /// If specified, will bind to the ipv6 interface if host is "any" instead of ipv4
    #[clap(short = '6', long)]
    pub use_ipv6: bool,

    /// Logic to apply to server when determining when to shutdown automatically
    ///
    /// 1. "never" means the server will never automatically shut down
    /// 2. "after=<N>" means the server will shut down after N seconds
    /// 3. "lonely=<N>" means the server will shut down after N seconds with no connections
    ///
    /// Default is to never shut down
    #[clap(long)]
    pub shutdown: Option<Shutdown>,

    /// Changes the current working directory (cwd) to the specified directory
    #[clap(long)]
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
