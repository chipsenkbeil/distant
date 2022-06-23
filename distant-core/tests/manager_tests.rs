use async_trait::async_trait;
use distant_core::{
    net::{
        AuthClient, FramedTransport, InmemoryTransport, IntoSplit, OneshotListener, PlainCodec,
        ServerRef,
    },
    ConnectHandler, Destination, DistantApiServer, DistantClient, DistantManager,
    DistantManagerClient, DistantManagerClientConfig, DistantManagerConfig, DistantRequestData,
    DistantResponseData, Extra,
};
use std::io;

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

/// Creates a [`DistantClient`] and [`DistantApiServer`] pair connected inmemory
fn setup_distant_client_server() -> (DistantClient, Box<dyn ServerRef>) {
    use distant_core::net::ServerExt;
    let (t1, t2) = FramedTransport::pair(100);
    (
        DistantClient::from_framed_transport(t1).unwrap(),
        DistantApiServer::local()
            .unwrap()
            .start(OneshotListener::from_value(t2.into_split()))
            .unwrap(),
    )
}

struct TestConnectHandler;

#[async_trait]
impl ConnectHandler for TestConnectHandler {
    async fn connect(
        &self,
        _destination: &Destination,
        _extra: &Extra,
        _auth: &AuthClient,
    ) -> io::Result<DistantClient> {
        let (client, _server) = setup_distant_client_server();
        Ok(client)
    }
}

#[tokio::test]
async fn should_be_able_to_establish_a_single_connection_and_communicate() {
    let (transport, listener) = setup().await;

    let config = DistantManagerConfig::default();
    let manager_ref = DistantManager::start(config, listener).expect("Failed to start manager");

    // NOTE: To pass in a raw function, we HAVE to specify the types of the parameters manually,
    //       otherwise we get a compilation error about lifetime mismatches
    manager_ref
        .register_connect_handler("scheme", TestConnectHandler)
        .await
        .expect("Failed to register handler");

    let config = DistantManagerClientConfig::with_empty_prompts();
    let mut client =
        DistantManagerClient::new(config, transport).expect("Failed to connect to manager");

    // Test establishing a connection to some remote server
    let id = client
        .connect(
            "scheme://host".parse::<Destination>().unwrap(),
            "key=value".parse::<Extra>().unwrap(),
        )
        .await
        .expect("Failed to connect to a remote server");

    // Test retrieving list of connections
    let list = client
        .list()
        .await
        .expect("Failed to get list of connections");
    assert_eq!(list.len(), 1);
    assert_eq!(list.get(&id).unwrap().to_string(), "scheme://host/");

    // Test retrieving information
    let info = client
        .info(id)
        .await
        .expect("Failed to get info about connection");
    assert_eq!(info.id, id);
    assert_eq!(info.destination.to_string(), "scheme://host/");
    assert_eq!(info.extra, "key=value".parse::<Extra>().unwrap());

    let response = client
        .send_single(id, DistantRequestData::SystemInfo {})
        .await
        .expect("Failed to get response to request");
    match response {
        DistantResponseData::SystemInfo { .. } => (),
        x => panic!("Got unexpected response: {:?}", x),
    }
}
