use crate::{constants::USERNAME, Config};
use clap::{Args, ValueEnum};
use derive_more::IsVariant;
use std::{io, net::IpAddr, path::PathBuf};

#[derive(Args, Debug)]
pub struct ServerListenConfig {
    /// If specified, launch will fail when attempting to bind to a unix socket that
    /// already exists, rather than removing the old socket
    #[clap(long)]
    pub fail_if_socket_exists: bool,

    /// The time in seconds before shutting down the server if there are no active
    /// connections. The countdown begins once all connections have closed and
    /// stops when a new connection is made. In not specified, the server will not
    /// shutdown at any point when there are no active connections.
    ///
    /// In the case of launch, this is only applicable when it is set to socket session
    /// as this controls when the unix socket listener would shutdown, not when the
    /// remote server it is connected to will shutdown.
    ///
    /// To configure the remote server's shutdown time, provide it as an argument
    /// via `--extra-server-args`
    #[clap(long)]
    pub shutdown_after: Option<f32>,

    /// When session is socket, runs in foreground instead of spawning a background process
    #[clap(long)]
    pub foreground: bool,

    /// Path to distant program on remote machine to execute via ssh;
    /// by default, this program needs to be available within PATH as
    /// specified when compiling ssh (not your login shell)
    #[clap(long, default_value = "distant")]
    pub distant: String,

    /// Path to ssh program on local machine to execute when using external ssh
    #[clap(long, default_value = "ssh")]
    pub ssh: String,

    /// If using native ssh integration, represents the backend
    #[cfg(any(feature = "libssh", feature = "ssh2"))]
    #[clap(long, default_value_t = distant_ssh2::SshBackend::default())]
    pub ssh_backend: distant_ssh2::SshBackend,

    /// If specified, will use the external ssh program to launch the server
    /// instead of the native integration; does nothing if the ssh2 feature is
    /// not enabled as there is no other option than external ssh
    #[clap(long)]
    pub external_ssh: bool,

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
    #[clap(long, value_name = "ssh|any|IP", default_value = "ssh")]
    pub bind_server: BindAddress,

    /// Additional arguments to provide to the server
    #[clap(long, allow_hyphen_values(true))]
    pub extra_server_args: Option<String>,

    /// Username to use when sshing into remote machine
    #[clap(short, long)]
    pub username: Option<String>,

    /// Explicit identity file to use with ssh
    #[clap(short, long)]
    pub identity_file: Option<PathBuf>,

    /// If specified, will not launch distant using a login shell but instead execute it directly
    #[clap(long)]
    pub no_shell: bool,

    /// Port to use for sshing into the remote machine
    #[clap(short, long, default_value = "22")]
    pub port: u16,
}

/// Represents options for binding a server to an IP address
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, IsVariant)]
pub enum BindAddress {
    #[display(fmt = "ssh")]
    Ssh,
    #[display(fmt = "any")]
    Any,
    Ip(IpAddr),
}
