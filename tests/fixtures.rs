use assert_cmd::Command;
use distant_core::*;
use rstest::*;
use std::{ffi::OsStr, net::SocketAddr, thread, time::Duration};
use tokio::{runtime::Runtime, sync::mpsc};

/// Timeout to wait for a command to complete
const TIMEOUT_SECS: u64 = 10;

/// Context for some listening distant server
pub struct DistantServerCtx {
    pub addr: SocketAddr,
    pub auth_key: String,
    done_tx: mpsc::Sender<()>,
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

    /// Attempts to shutdown the server if it is not already dead
    pub fn shutdown(&self) {
        let _ = self.done_tx.send(());
    }
}

impl Drop for DistantServerCtx {
    /// Kills server upon drop
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[fixture]
pub fn ctx() -> DistantServerCtx {
    let ip_addr = "127.0.0.1".parse().unwrap();
    let (done_tx, mut done_rx) = mpsc::channel(1);
    let (started_tx, mut started_rx) = mpsc::channel(1);

    // NOTE: We spawn a dedicated thread that runs our tokio runtime separately
    //       from our test itself because using assert_cmd blocks the thread
    //       and prevents our runtime from working unless we make the tokio
    //       test multi-threaded using `tokio::test(flavor = "multi_thread", worker_threads = 1)`
    //       which isn't great because we're only using async tests for our
    //       server itself; so, we hide that away since our test logic doesn't need to be async
    thread::spawn(move || match Runtime::new() {
        Ok(rt) => {
            println!("Starting...");
            rt.block_on(async move {
                println!("Async...");
                let server = DistantServer::bind(ip_addr, "0".parse().unwrap(), None, 100)
                    .await
                    .unwrap();

                started_tx
                    .send(Ok((server.port(), server.to_unprotected_hex_auth_key())))
                    .await
                    .unwrap();

                let _ = done_rx.recv().await;
            });
        }
        Err(x) => {
            started_tx.blocking_send(Err(x)).unwrap();
        }
    });

    // Extract our server startup data if we succeeded
    let (port, auth_key) = started_rx.blocking_recv().unwrap().unwrap();

    DistantServerCtx {
        addr: SocketAddr::new(ip_addr, port),
        auth_key,
        done_tx,
    }
}
