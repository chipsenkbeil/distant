use async_trait::async_trait;
use distant_core::{
    net::{
        AuthClient, FramedTransport, InmemoryTransport, OneshotListener, PlainCodec, Request,
        Response, UntypedTransportRead, UntypedTransportWrite,
    },
    ConnectHandler, Destination, DistantClient, DistantManager, DistantManagerClient,
    DistantManagerClientConfig, DistantManagerConfig, DistantMsg, DistantRequestData,
    DistantResponseData, Extra,
};
use std::{io, path::PathBuf};
use tokio::sync::mpsc;

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

/// Creates a [`DistantClient`] with a connected transport
fn setup_distant_client() -> (
    DistantClient,
    FramedTransport<InmemoryTransport, PlainCodec>,
) {
    let (t1, t2) = FramedTransport::pair(100);
    (DistantClient::from_framed_transport(t1).unwrap(), t2)
}

struct TestConnectHandler(mpsc::Sender<FramedTransport<InmemoryTransport, PlainCodec>>);

#[async_trait]
impl ConnectHandler for TestConnectHandler {
    async fn connect(
        &self,
        _destination: &Destination,
        _extra: &Extra,
        _auth: &AuthClient,
    ) -> io::Result<DistantClient> {
        let (client, transport) = setup_distant_client();
        self.0.send(transport).await.unwrap();
        Ok(client)
    }
}

#[tokio::test]
async fn should_be_able_to_establish_a_single_connection_and_communicate() {
    let (transport, listener) = setup().await;

    let config = DistantManagerConfig::default();
    let manager_ref = DistantManager::start(config, listener).expect("Failed to start manager");

    let (tx, mut rx) = mpsc::channel(100);

    // NOTE: To pass in a raw function, we HAVE to specify the types of the parameters manually,
    //       otherwise we get a compilation error about lifetime mismatches
    manager_ref
        .register_connect_handler("scheme", TestConnectHandler(tx))
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

    // Test forwarding a request, capturing it with our transport, and sending back a response
    tokio::spawn(async move {
        eprintln!("Getting transport");
        let mut transport = rx.recv().await.unwrap();
        eprintln!("Reading request");
        let request = transport
            .read::<Request<DistantMsg<DistantRequestData>>>()
            .await
            .unwrap()
            .unwrap();

        eprintln!("Request {}", request.id);
        let id = request.id;
        let path = match request.payload.into_single() {
            Some(DistantRequestData::FileReadText { path }) => path,
            x => panic!("Got unexpected request: {:?}", x),
        };

        eprintln!("Path {:?}", path);
        transport
            .write(Response::new(
                id,
                DistantMsg::Single(DistantResponseData::Text {
                    data: path.to_string_lossy().to_string(),
                }),
            ))
            .await
            .unwrap();
        eprintln!("Sent response");
    });

    eprintln!("SEND");
    let response = client
        .send_single(
            id,
            DistantRequestData::FileReadText {
                path: PathBuf::from("some_path"),
            },
        )
        .await
        .expect("Failed to get response to request");
    match response {
        DistantResponseData::Text { data } => assert_eq!(data, "some_path"),
        x => panic!("Got unexpected response: {:?}", x),
    }
}
