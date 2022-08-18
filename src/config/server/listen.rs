use anyhow::Context;
use clap::Args;
use derive_more::Display;
use distant_core::{
    net::{PortRange, Shutdown},
    Map,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    net::{AddrParseError, IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    str::FromStr,
};

#[derive(Args, Debug, Default, Serialize, Deserialize)]
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

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindAddress {
    #[display = "ssh"]
    Ssh,
    #[display = "any"]
    Any,
    Ip(IpAddr),
}

impl FromStr for BindAddress {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Ok(if s.eq_ignore_ascii_case("ssh") {
            Self::Ssh
        } else if s.eq_ignore_ascii_case("any") {
            Self::Any
        } else {
            s.parse()?
        })
    }
}

impl BindAddress {
    /// Resolves address into valid IP; in the case of "any", will leverage the
    /// `use_ipv6` flag to determine if binding should use ipv4 or ipv6
    pub fn resolve(self, use_ipv6: bool) -> anyhow::Result<IpAddr> {
        match self {
            Self::Ssh => {
                let ssh_connection =
                    env::var("SSH_CONNECTION").context("Failed to read SSH_CONNECTION")?;
                let ip_str = ssh_connection.split(' ').nth(2).ok_or_else(|| {
                    anyhow::anyhow!("SSH_CONNECTION missing 3rd argument (host ip)")
                })?;
                let ip = ip_str
                    .parse::<IpAddr>()
                    .context("Failed to parse IP address")?;
                Ok(ip)
            }
            Self::Any if use_ipv6 => Ok(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            Self::Any => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Self::Ip(addr) => Ok(addr),
        }
    }
}
