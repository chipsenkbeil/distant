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
                identity_file: map
                    .remove("ssh.identity_file")
                    .and_then(|x| x.parse::<PathBuf>().ok()),
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

            if let Some(x) = config.ssh.identity_file {
                this.insert(
                    "ssh.identity_file".to_string(),
                    x.to_string_lossy().to_string(),
                );
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

    /// Explicit identity file to use with ssh
    #[clap(name = "ssh-identity-file", short = 'i', long)]
    pub identity_file: Option<PathBuf>,
}
