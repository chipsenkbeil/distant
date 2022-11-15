use distant_core::net::client::{Client, TcpConnector};
use distant_core::net::common::authentication::{DummyAuthHandler, Verifier};
use distant_core::net::common::PortRange;
use distant_core::net::server::Server;
use distant_core::{DistantApiServerHandler, DistantClient, LocalDistantApi};
use rstest::*;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct DistantClientCtx {
    pub client: DistantClient,
    _done_tx: mpsc::Sender<()>,
}

impl DistantClientCtx {
    pub async fn initialize() -> Self {
        let ip_addr = "127.0.0.1".parse().unwrap();
        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);
        let (started_tx, mut started_rx) = mpsc::channel::<u16>(1);

        tokio::spawn(async move {
            if let Ok(api) = LocalDistantApi::initialize(Default::default()) {
                let port: PortRange = "0".parse().unwrap();
                let port = {
                    let handler = DistantApiServerHandler::new(api);
                    let server_ref = Server::new()
                        .handler(handler)
                        .verifier(Verifier::none())
                        .into_tcp_builder()
                        .start(ip_addr, port)
                        .await
                        .unwrap();
                    server_ref.port()
                };

                started_tx.send(port).await.unwrap();
                let _ = done_rx.recv().await;
            }
        });

        // Extract our server startup data if we succeeded
        let port = started_rx.recv().await.unwrap();

        // Now initialize our client
        let client: DistantClient = Client::build()
            .auth_handler(DummyAuthHandler)
            .timeout(Duration::from_secs(1))
            .connector(TcpConnector::new(
                format!("{}:{}", ip_addr, port)
                    .parse::<SocketAddr>()
                    .unwrap(),
            ))
            .connect()
            .await
            .unwrap();

        DistantClientCtx {
            client,
            _done_tx: done_tx,
        }
    }
}

#[fixture]
pub async fn ctx() -> DistantClientCtx {
    DistantClientCtx::initialize().await
}
