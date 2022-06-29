use crate::config::BindAddress;
use crate::Merge;
use clap::Args;
use distant_core::net::PortRange;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ServerListenConfig {
    /// Runs in foreground instead of spawning a background process
    #[clap(long)]
    pub foreground: bool,

    /// Control the IP address that the distant binds to
    ///
    /// There are three options here:
    ///
    /// 1. `ssh`: the server will reply from the IP address that the SSH
    /// connection came from (as found in the SSH_CONNECTION environment variable). This is
    /// useful for multihomed servers.
    ///
    /// 2. `any`: the server will reply on the default interface and will not bind to
    /// a particular IP address. This can be useful if the connection is made through sslh or
    /// another tool that makes the SSH connection appear to come from localhost.
    ///
    /// 3. `IP`: the server will attempt to bind to the specified IP address.
    #[clap(short, long, value_name = "ssh|any|IP")]
    pub host: Option<BindAddress>,

    /// If specified, will bind to the ipv6 interface if host is "any" instead of ipv4
    #[clap(short = '6', long)]
    pub use_ipv6: bool,

    /// If specified, the server will not generate a key but instead listen on stdin for the next
    /// 32 bytes that it will use as the key instead. Receiving less than 32 bytes before stdin
    /// is closed is considered an error and any bytes after the first 32 are not used for the key
    #[clap(long)]
    pub key_from_stdin: bool,

    /// The time in seconds before shutting down the server if there are no active
    /// connections. The countdown begins once all connections have closed and
    /// stops when a new connection is made. In not specified, the server will not
    /// shutdown at any point when there are no active connections.
    #[clap(long)]
    pub shutdown_after: Option<f32>,

    /// Changes the current working directory (cwd) to the specified directory
    #[clap(long)]
    pub current_dir: Option<PathBuf>,

    /// Set the port(s) that the server will attempt to bind to
    ///
    /// This can be in the form of PORT1 or PORT1:PORTN to provide a range of ports.
    /// With -p 0, the server will let the operating system pick an available TCP port.
    ///
    /// Please note that this option does not affect the server-side port used by SSH
    #[clap(short, long, value_name = "PORT[:PORT2]")]
    pub port: Option<PortRange>,
}

impl Merge for ServerListenConfig {
    fn merge(&mut self, other: Self) {
        self.foreground = other.foreground;
        self.use_ipv6 = other.use_ipv6;
        self.key_from_stdin = other.key_from_stdin;

        if let Some(x) = other.host {
            self.host = Some(x);
        }
        if let Some(x) = other.shutdown_after {
            self.shutdown_after = Some(x);
        }
        if let Some(x) = other.current_dir {
            self.current_dir = Some(x);
        }
        if let Some(x) = other.port {
            self.port = Some(x);
        }
    }
}
