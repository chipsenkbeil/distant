use crate::stress::utils;
use distant_core::{DistantApiServer, DistantClient, LocalDistantApi};
use distant_net::{
    PortRange, SecretKey, SecretKey32, TcpClientExt, TcpServerExt, XChaCha20Poly1305Codec,
};
use rstest::*;
use std::time::Duration;
use tokio::sync::mpsc;

const LOG_PATH: &str = "/tmp/test.distant.server.log";

pub struct DistantClientCtx {
    pub client: DistantClient,
    _done_tx: mpsc::Sender<()>,
}

impl DistantClientCtx {
    pub async fn initialize() -> Self {
        let ip_addr = "127.0.0.1".parse().unwrap();
        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);
        let (started_tx, mut started_rx) = mpsc::channel::<(u16, SecretKey32)>(1);

        tokio::spawn(async move {
            let logger = utils::init_logging(LOG_PATH);
            let key = SecretKey::default();
            let codec = XChaCha20Poly1305Codec::from(key.clone());

            if let Ok(api) = LocalDistantApi::initialize(Default::default()) {
                let port: PortRange = "0".parse().unwrap();
                let port = {
                    let server_ref = DistantApiServer::new(api)
                        .start(ip_addr, port, codec)
                        .await
                        .unwrap();
                    server_ref.port()
                };

                started_tx.send((port, key)).await.unwrap();
                let _ = done_rx.recv().await;
            }

            logger.flush();
            logger.shutdown();
        });

        // Extract our server startup data if we succeeded
        let (port, key) = started_rx.recv().await.unwrap();

        // Now initialize our client
        let client = DistantClient::connect_timeout(
            format!("{}:{}", ip_addr, port).parse().unwrap(),
            XChaCha20Poly1305Codec::from(key),
            Duration::from_secs(1),
        )
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
