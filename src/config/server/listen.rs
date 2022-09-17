use anyhow::Context;
use clap::Args;
use distant_core::{
    net::{PortRange, Shutdown},
    Host, HostParseError, Map,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindAddress {
    /// Should read address from `SSH_CONNECTION` environment variable, which contains four
    /// space-separated values:
    ///
    /// * client IP address
    /// * client port number
    /// * server IP address
    /// * server port number
    Ssh,

    /// Should bind to `0.0.0.0` or `::` depending on ipv6 flag
    Any,

    /// Should bind to the specified host, which could be `example.com`, `localhost`, or an IP
    /// address like `203.0.113.1` or `2001:DB8::1`
    Host(Host),
}

impl fmt::Display for BindAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Ssh => write!(f, "ssh"),
            Self::Any => write!(f, "any"),
            Self::Host(host) => write!(f, "{host}"),
        }
    }
}

impl FromStr for BindAddress {
    type Err = HostParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Ok(if s.eq_ignore_ascii_case("ssh") {
            Self::Ssh
        } else if s.eq_ignore_ascii_case("any") {
            Self::Any
        } else {
            Self::Host(s.parse::<Host>()?)
        })
    }
}

impl BindAddress {
    /// Resolves address into valid IP; in the case of "any", will leverage the
    /// `use_ipv6` flag to determine if binding should use ipv4 or ipv6
    pub async fn resolve(self, use_ipv6: bool) -> anyhow::Result<IpAddr> {
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
            Self::Host(host) => match host {
                Host::Ipv4(x) => Ok(IpAddr::V4(x)),
                Host::Ipv6(x) => Ok(IpAddr::V6(x)),

                // Attempt to resolve the hostname, tacking on a :80 as a port if no colon is found
                // in the hostname as we MUST have a socket address like example.com:80 in order to
                // perform dns resolution
                Host::Name(x) => Ok(tokio::net::lookup_host(if x.contains(':') {
                    x.to_string()
                } else {
                    format!("{x}:80")
                })
                .await?
                .map(|addr| addr.ip())
                .find(|ip| (use_ipv6 && ip.is_ipv6()) || (!use_ipv6 && ip.is_ipv4()))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unable to resolve {x} to {} address",
                        if use_ipv6 { "ipv6" } else { "ipv4" }
                    )
                })?),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_string_should_properly_print_bind_address() {
        assert_eq!(BindAddress::Any.to_string(), "any");
        assert_eq!(BindAddress::Ssh.to_string(), "ssh");
        assert_eq!(
            BindAddress::Host(Host::Ipv4(Ipv4Addr::new(203, 0, 113, 1))).to_string(),
            "203.0.113.1"
        );
        assert_eq!(
            BindAddress::Host(Host::Ipv6(Ipv6Addr::new(
                0x2001, 0x0DB8, 0, 0, 0, 0, 0, 0x0001
            )))
            .to_string(),
            "2001:db8::1"
        );
        assert_eq!(
            BindAddress::Host(Host::Name(String::from("example.com"))).to_string(),
            "example.com"
        );
    }

    #[test]
    fn parse_should_correctly_parse_host_or_special_cases() {
        assert_eq!("any".parse::<BindAddress>().unwrap(), BindAddress::Any);
        assert_eq!("ssh".parse::<BindAddress>().unwrap(), BindAddress::Ssh);
        assert_eq!(
            "203.0.113.1".parse::<BindAddress>().unwrap(),
            BindAddress::Host(Host::Ipv4(Ipv4Addr::new(203, 0, 113, 1)))
        );
        assert_eq!(
            "2001:DB8::1".parse::<BindAddress>().unwrap(),
            BindAddress::Host(Host::Ipv6(Ipv6Addr::new(
                0x2001, 0x0DB8, 0, 0, 0, 0, 0, 0x0001
            )))
        );
        assert_eq!(
            "example.com".parse::<BindAddress>().unwrap(),
            BindAddress::Host(Host::Name(String::from("example.com")))
        );
        assert_eq!(
            "localhost".parse::<BindAddress>().unwrap(),
            BindAddress::Host(Host::Name(String::from("localhost")))
        );
    }

    #[tokio::test]
    async fn resolve_should_properly_resolve_bind_address() {
        // For ssh, we check SSH_CONNECTION, and there are three situations where this can fail:
        //
        // 1. The environment variable does not exist
        // 2. The environment variable does not have at least 3 (out of 4) space-separated args
        // 3. The environment variable has an invalid IP address as the 3 arg
        BindAddress::Ssh.resolve(false).await.unwrap_err();

        env::set_var("SSH_CONNECTION", "127.0.0.1 1234");
        BindAddress::Ssh.resolve(false).await.unwrap_err();

        env::set_var("SSH_CONNECTION", "127.0.0.1 1234 -notaddress 1234");
        BindAddress::Ssh.resolve(false).await.unwrap_err();

        env::set_var("SSH_CONNECTION", "127.0.0.1 1234 127.0.0.1 1234");
        assert_eq!(
            BindAddress::Ssh.resolve(false).await.unwrap(),
            Ipv4Addr::new(127, 0, 0, 1)
        );

        // Any will resolve to unspecified ipv4 if the ipv6 flag is false
        assert_eq!(
            BindAddress::Any.resolve(false).await.unwrap(),
            Ipv4Addr::UNSPECIFIED
        );

        // Any will resolve to unspecified ipv6 if the ipv6 flag is true
        assert_eq!(
            BindAddress::Any.resolve(true).await.unwrap(),
            Ipv6Addr::UNSPECIFIED
        );

        // Host with ipv4 address should return that address
        assert_eq!(
            BindAddress::Host(Host::Ipv4(Ipv4Addr::UNSPECIFIED))
                .resolve(false)
                .await
                .unwrap(),
            Ipv4Addr::UNSPECIFIED
        );

        // Host with ipv6 address should return that address
        assert_eq!(
            BindAddress::Host(Host::Ipv6(Ipv6Addr::UNSPECIFIED))
                .resolve(false)
                .await
                .unwrap(),
            Ipv6Addr::UNSPECIFIED
        );

        // Host with name should attempt to resolve the name
        assert_eq!(
            BindAddress::Host(Host::Name(String::from("example.com")))
                .resolve(false)
                .await
                .unwrap(),
            tokio::net::lookup_host("example.com:80")
                .await
                .unwrap()
                .next()
                .unwrap()
                .ip(),
        );

        // Should support resolving localhost using ipv4/ipv6
        assert_eq!(
            BindAddress::Host(Host::Name(String::from("localhost")))
                .resolve(false)
                .await
                .unwrap(),
            Ipv4Addr::LOCALHOST
        );
        assert_eq!(
            BindAddress::Host(Host::Name(String::from("localhost")))
                .resolve(true)
                .await
                .unwrap(),
            Ipv6Addr::LOCALHOST
        );
    }
}
