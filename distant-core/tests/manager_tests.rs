use distant_core::{
    net::{FramedTransport, InmemoryTransport, IntoSplit, OneshotListener, PlainCodec},
    BoxedDistantReader, BoxedDistantWriter, Destination, DistantApiServer, DistantChannelExt,
    DistantManager, DistantManagerClient, DistantManagerClientConfig, DistantManagerConfig, Extra,
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

#[tokio::test]
async fn should_be_able_to_establish_a_single_connection_and_communicate() {
    let (transport, listener) = setup().await;

    let config = DistantManagerConfig::default();
    let manager_ref = DistantManager::start(config, listener).expect("Failed to start manager");

    // NOTE: To pass in a raw function, we HAVE to specify the types of the parameters manually,
    //       otherwise we get a compilation error about lifetime mismatches
    manager_ref
        .register_connect_handler("scheme", |_: &_, _: &_, _: &mut _| async {
            use distant_core::net::ServerExt;
            let (t1, t2) = FramedTransport::pair(100);

            // Spawn a server on one end
            let _ = DistantApiServer::local()
                .unwrap()
                .start(OneshotListener::from_value(t2.into_split()))?;

            // Create a reader/writer pair on the other end
            let (writer, reader) = t1.into_split();
            let writer: BoxedDistantWriter = Box::new(writer);
            let reader: BoxedDistantReader = Box::new(reader);
            Ok((writer, reader))
        })
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

    // Create a new channel and request some data
    let mut channel = client
        .open_channel(id)
        .await
        .expect("Failed to open channel");
    let _ = channel
        .system_info()
        .await
        .expect("Failed to get system information");

    // Test killing a connection
    let _ = client.kill(id).await.expect("Failed to kill connection");

    // Test getting an error to ensure that serialization of that data works,
    // which we do by trying to access a connection that no longer exists
    let err = client.info(id).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
}
