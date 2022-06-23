use distant_core::{
    net::{FramedTransport, InmemoryTransport, OneshotListener, PlainCodec},
    DistantClient, DistantManager, DistantManagerClient, DistantManagerClientConfig,
    DistantManagerConfig,
};

/// Creates a client transport and server listener for our tests
/// that are connected together
async fn setup() -> (
    FramedTransport<InmemoryTransport, PlainCodec>,
    OneshotListener<FramedTransport<InmemoryTransport, PlainCodec>>,
) {
    let (t1, t2) = InmemoryTransport::pair(100);

    let listener = OneshotListener::from_value(FramedTransport::new(t2, PlainCodec));
    let transport = FramedTransport::new(t1, PlainCodec);
    (transport, listener)
}

/// Creates a dummy [`DistantClient`]
fn dummy_distant_client() -> DistantClient {
    setup_distant_client().0
}

/// Creates a [`DistantClient`] with a connected transport
fn setup_distant_client() -> (
    DistantClient,
    FramedTransport<InmemoryTransport, PlainCodec>,
) {
    let (t1, t2) = FramedTransport::pair(1);
    (DistantClient::from_framed_transport(t1).unwrap(), t2)
}

#[tokio::test]
async fn should_be_able_to_manage_a_single_connection() {
    let (transport, listener) = setup().await;

    let config = DistantManagerConfig::default();
    let manager_ref = DistantManager::start(config, listener).expect("Failed to start manager");
    manager_ref
        .register_connect_handler("scheme", |_destination, _extra, _auth| async {
            Ok(dummy_distant_client())
        })
        .await
        .expect("Failed to register handler");

    let config = DistantManagerClientConfig::with_empty_prompts();
    let client =
        DistantManagerClient::new(config, transport).expect("Failed to connect to manager");
}

#[tokio::test]
async fn should_be_able_to_manage_multiple_connections() {
    todo!();
}
