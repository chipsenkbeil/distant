use super::common::BindAddress;
use clap::Args;
use distant_core::net::common::Map;
use serde::{Deserialize, Serialize};

#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientLaunchConfig {
    #[clap(flatten)]
    #[serde(flatten)]
    pub distant: ClientLaunchDistantConfig,

    /// Additional options to provide, typically forwarded to the handler within the manager
    /// facilitating the launch of a distant server. Options are key-value pairs separated by
    /// comma.
    ///
    /// E.g. `key="value",key2="value2"`
    #[clap(long, default_value_t)]
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

#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientLaunchDistantConfig {
    /// Path to distant program on remote machine to execute via ssh;
    /// by default, this program needs to be available within PATH as
    /// specified when compiling ssh (not your login shell)
    #[clap(name = "distant", long)]
    pub bin: Option<String>,

    /// Control the IP address that the server binds to.
    ///
    /// The default is `ssh', in which case the server will reply from the IP address that the SSH
    /// connection came from (as found in the SSH_CONNECTION environment variable). This is
    /// useful for multihomed servers.
    ///
    /// With --bind-server=any, the server will reply on the default interface and will not bind to
    /// a particular IP address. This can be useful if the connection is made through sslh or
    /// another tool that makes the SSH connection appear to come from localhost.
    ///
    /// With --bind-server=IP, the server will attempt to bind to the specified IP address.
    #[clap(name = "distant-bind-server", long, value_name = "ssh|any|IP")]
    pub bind_server: Option<BindAddress>,

    /// Additional arguments to provide to the server
    #[clap(name = "distant-args", long, allow_hyphen_values(true))]
    pub args: Option<String>,
}
