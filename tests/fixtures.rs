use assert_cmd::Command;
use distant_core::*;
use rstest::*;
use std::{ffi::OsStr, net::SocketAddr, time::Duration};

/// Timeout to wait for a command to complete
const TIMEOUT_SECS: u64 = 10;

/// Context for some listening distant server
pub struct DistantServerCtx {
    pub addr: SocketAddr,
    pub auth_key: String,
    pub server: DistantServer,
}

impl DistantServerCtx {
    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
    pub fn new_cmd(&self, subcommand: impl AsRef<OsStr>) -> Command {
        let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap();

        println!("DISTANT_HOST = {}", self.addr.ip());
        println!("DISTANT_PORT = {}", self.addr.port());
        println!("DISTANT_AUTH_KEY = {}", self.auth_key);

        // NOTE: We define a command that has a timeout of 10s because the handshake
        //       involved in a non-release test can take several seconds
        cmd.arg(subcommand)
            .args(&["--session", "environment"])
            .env("DISTANT_HOST", self.addr.ip().to_string())
            .env("DISTANT_PORT", self.addr.port().to_string())
            .env("DISTANT_AUTH_KEY", self.auth_key.as_str())
            .timeout(Duration::from_secs(TIMEOUT_SECS));
        cmd
    }
}

impl Drop for DistantServerCtx {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[fixture]
pub async fn ctx() -> DistantServerCtx {
    let ip_addr = "127.0.0.1".parse().unwrap();
    let server = DistantServer::bind(ip_addr, "0".parse().unwrap(), None, 100)
        .await
        .unwrap();

    DistantServerCtx {
        addr: SocketAddr::new(ip_addr, server.port()),
        auth_key: server.to_unprotected_hex_auth_key(),
        server,
    }
}
