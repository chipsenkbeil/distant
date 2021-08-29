use assert_cmd::Command;
use distant_core::*;
use rstest::*;
use std::{ffi::OsStr, net::SocketAddr, thread};
use tokio::{runtime::Runtime, sync::mpsc};

/// Context for some listening distant server
pub struct DistantServerCtx {
    pub addr: SocketAddr,
    pub auth_key: String,
    done_tx: mpsc::Sender<()>,
}

impl DistantServerCtx {
    pub fn initialize() -> Self {
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
                rt.block_on(async move {
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

        Self {
            addr: SocketAddr::new(ip_addr, port),
            auth_key,
            done_tx,
        }
    }

    /// Produces a new test command that configures some distant command
    /// configured with an environment that can talk to a remote distant server
    pub fn new_cmd(&self, subcommand: impl AsRef<OsStr>) -> Command {
        let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap();
        cmd.arg(subcommand)
            .args(&["--session", "environment"])
            .env("DISTANT_HOST", self.addr.ip().to_string())
            .env("DISTANT_PORT", self.addr.port().to_string())
            .env("DISTANT_AUTH_KEY", self.auth_key.as_str());
        cmd
    }
}

impl Drop for DistantServerCtx {
    /// Kills server upon drop
    fn drop(&mut self) {
        let _ = self.done_tx.send(());
    }
}

#[fixture]
pub fn ctx() -> &'static DistantServerCtx {
    lazy_static::lazy_static! {
        static ref CTX: DistantServerCtx = DistantServerCtx::initialize();
    }

    &CTX
}

#[fixture]
pub fn action_cmd(ctx: &'_ DistantServerCtx) -> Command {
    ctx.new_cmd("action")
}

#[fixture]
pub fn lsp_cmd(ctx: &'_ DistantServerCtx) -> Command {
    ctx.new_cmd("lsp")
}
