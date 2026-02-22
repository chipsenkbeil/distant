use std::net::SocketAddr;
use std::time::Duration;

use distant_core::net::auth::{DummyAuthHandler, Verifier};
use distant_core::net::client::{Client as NetClient, TcpConnector};
use distant_core::net::common::PortRange;
use distant_core::net::server::Server;
use distant_core::{ApiServerHandler, Client};
use distant_host::Api;
use rstest::*;
use tokio::sync::mpsc;

pub struct ClientCtx {
    pub client: Client,
    _done_tx: mpsc::Sender<()>,
}

impl ClientCtx {
    pub async fn initialize() -> Self {
        let ip_addr = "127.0.0.1".parse().unwrap();
        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);
        let (started_tx, mut started_rx) = mpsc::channel::<u16>(1);

        tokio::spawn(async move {
            if let Ok(api) = Api::initialize(Default::default()) {
                let port: PortRange = "0".parse().unwrap();
                let port = {
                    let handler = ApiServerHandler::new(api);
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
        let client: Client = NetClient::build()
            .auth_handler(DummyAuthHandler)
            .connect_timeout(Duration::from_secs(1))
            .connector(TcpConnector::new(
                format!("{}:{}", ip_addr, port)
                    .parse::<SocketAddr>()
                    .unwrap(),
            ))
            .connect()
            .await
            .unwrap();

        ClientCtx {
            client,
            _done_tx: done_tx,
        }
    }
}

#[fixture]
pub async fn ctx() -> ClientCtx {
    ClientCtx::initialize().await
}
