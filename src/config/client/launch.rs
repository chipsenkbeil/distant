use crate::{config::BindAddress, Merge};
use clap::Args;
use distant_core::Extra;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientLaunchConfig {
    #[clap(flatten)]
    #[serde(flatten)]
    pub distant: ClientLaunchDistantConfig,

    #[clap(flatten)]
    #[serde(flatten)]
    pub ssh: ClientLaunchSshConfig,
}

impl From<Extra> for ClientLaunchConfig {
    fn from(mut extra: Extra) -> Self {
        Self {
            distant: ClientLaunchDistantConfig {
                bin: extra.remove("distant.bin"),
                bind_server: extra
                    .remove("distant.bind_server")
                    .and_then(|x| x.parse::<BindAddress>().ok()),
                args: extra.remove("distant.args"),
                no_shell: extra
                    .remove("distant.no_shell")
                    .and_then(|x| x.parse::<bool>().ok())
                    .unwrap_or_default(),
            },
            ssh: ClientLaunchSshConfig {
                bin: extra.remove("ssh.bind"),
                #[cfg(any(feature = "libssh", feature = "ssh2"))]
                backend: extra
                    .remove("ssh.backend")
                    .and_then(|x| x.parse::<distant_ssh2::SshBackend>().ok()),
                external: extra
                    .remove("ssh.external")
                    .and_then(|x| x.parse::<bool>().ok())
                    .unwrap_or_default(),
                username: extra.remove("ssh.username"),
                identity_file: extra
                    .remove("ssh.identity_file")
                    .and_then(|x| x.parse::<PathBuf>().ok()),
                port: extra.remove("ssh.port").and_then(|x| x.parse::<u16>().ok()),
            },
        }
    }
}

impl From<ClientLaunchConfig> for Extra {
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

        this.insert(
            "distant.no_shell".to_string(),
            config.distant.no_shell.to_string(),
        );

        if let Some(x) = config.ssh.bin {
            this.insert("ssh.bin".to_string(), x);
        }

        #[cfg(any(feature = "libssh", feature = "ssh2"))]
        if let Some(x) = config.ssh.backend {
            this.insert("ssh.backend".to_string(), x.to_string());
        }

        this.insert("ssh.external".to_string(), config.ssh.external.to_string());

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

        this
    }
}

impl Merge for ClientLaunchConfig {
    fn merge(&mut self, other: Self) {
        self.distant.merge(other.distant);
        self.ssh.merge(other.ssh);
    }
}

impl Merge<ClientLaunchDistantConfig> for ClientLaunchConfig {
    fn merge(&mut self, other: ClientLaunchDistantConfig) {
        self.distant.merge(other);
    }
}

impl Merge<ClientLaunchSshConfig> for ClientLaunchConfig {
    fn merge(&mut self, other: ClientLaunchSshConfig) {
        self.ssh.merge(other);
    }
}

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientLaunchDistantConfig {
    /// Path to distant program on remote machine to execute via ssh;
    /// by default, this program needs to be available within PATH as
    /// specified when compiling ssh (not your login shell)
    #[clap(long = "distant")]
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
    #[clap(long = "distant-bind-server", value_name = "ssh|any|IP")]
    pub bind_server: Option<BindAddress>,

    /// Additional arguments to provide to the server
    #[clap(long = "distant-args", allow_hyphen_values(true))]
    pub args: Option<String>,

    /// If specified, will not launch distant using a login shell but instead execute it directly
    #[clap(long)]
    pub no_shell: bool,
}

impl Merge for ClientLaunchDistantConfig {
    fn merge(&mut self, other: Self) {
        self.no_shell = other.no_shell;

        if let Some(x) = other.bin {
            self.bin = Some(x);
        }
        if let Some(x) = other.bind_server {
            self.bind_server = Some(x);
        }
        if let Some(x) = other.args {
            self.args = Some(x);
        }
    }
}

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientLaunchSshConfig {
    /// Path to ssh program on local machine to execute when using external ssh
    #[clap(long = "ssh")]
    pub bin: Option<String>,

    /// If using native ssh integration, represents the backend
    #[cfg(any(feature = "libssh", feature = "ssh2"))]
    #[clap(long = "ssh-backend")]
    pub backend: Option<distant_ssh2::SshBackend>,

    /// If specified, will use the external ssh program to launch the server
    /// instead of the native integration; does nothing if the ssh2 feature is
    /// not enabled as there is no other option than external ssh
    #[clap(long = "ssh-external")]
    pub external: bool,

    /// Username to use when sshing into remote machine
    #[clap(short, long = "ssh-username")]
    pub username: Option<String>,

    /// Explicit identity file to use with ssh
    #[clap(short, long = "ssh-identity-file")]
    pub identity_file: Option<PathBuf>,

    /// Port to use for sshing into the remote machine
    #[clap(short, long = "ssh-port")]
    pub port: Option<u16>,
}

impl Merge for ClientLaunchSshConfig {
    fn merge(&mut self, other: Self) {
        self.external = other.external;

        if let Some(x) = other.bin {
            self.bin = Some(x);
        }
        if let Some(x) = other.backend {
            self.backend = Some(x);
        }
        if let Some(x) = other.username {
            self.username = Some(x);
        }
        if let Some(x) = other.identity_file {
            self.identity_file = Some(x);
        }
        if let Some(x) = other.port {
            self.port = Some(x);
        }
    }
}
