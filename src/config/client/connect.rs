use clap::Args;
use distant_core::Map;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientConnectConfig {
    #[cfg(any(feature = "libssh", feature = "ssh2"))]
    #[clap(flatten)]
    #[serde(flatten)]
    pub ssh: ClientConnectSshConfig,
}

impl From<Map> for ClientConnectConfig {
    fn from(mut map: Map) -> Self {
        Self {
            #[cfg(any(feature = "libssh", feature = "ssh2"))]
            ssh: ClientConnectSshConfig {
                backend: map
                    .remove("ssh.backend")
                    .and_then(|x| x.parse::<distant_ssh2::SshBackend>().ok()),
                username: map.remove("ssh.username"),
                identity_file: map
                    .remove("ssh.identity_file")
                    .and_then(|x| x.parse::<PathBuf>().ok()),
                port: map.remove("ssh.port").and_then(|x| x.parse::<u16>().ok()),
            },
        }
    }
}

impl From<ClientConnectConfig> for Map {
    fn from(config: ClientConnectConfig) -> Self {
        let mut this = Self::new();

        #[cfg(any(feature = "libssh", feature = "ssh2"))]
        {
            if let Some(x) = config.ssh.backend {
                this.insert("ssh.backend".to_string(), x.to_string());
            }

            if let Some(x) = config.ssh.username {
                this.insert("ssh.username".to_string(), x);
            }

            if let Some(x) = config.ssh.identity_file {
                this.insert(
                    "ssh.identity_file".to_string(),
                    x.to_string_lossy().to_string(),
                );
            }

            if let Some(x) = config.ssh.port {
                this.insert("ssh.port".to_string(), x.to_string());
            }
        }

        this
    }
}

#[cfg(any(feature = "libssh", feature = "ssh2"))]
#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientConnectSshConfig {
    /// Represents the backend
    #[clap(name = "ssh-backend", long)]
    pub backend: Option<distant_ssh2::SshBackend>,

    /// Username to use when sshing into remote machine
    #[clap(name = "ssh-username", short = 'u', long)]
    pub username: Option<String>,

    /// Explicit identity file to use with ssh
    #[clap(name = "ssh-identity-file", short = 'i', long)]
    pub identity_file: Option<PathBuf>,

    /// Port to use for sshing into the remote machine
    #[clap(name = "ssh-port", short = 'p', long)]
    pub port: Option<u16>,
}
