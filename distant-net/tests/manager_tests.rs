use async_trait::async_trait;
use distant_net::boxed_connect_handler;
use distant_net::client::{Client, ReconnectStrategy};
use distant_net::common::authentication::{DummyAuthHandler, Verifier};
use distant_net::common::{Destination, InmemoryTransport, Map, OneshotListener};
use distant_net::manager::{Config, ManagerClient, ManagerServer};
use distant_net::server::{Server, ServerCtx, ServerHandler};
use log::*;
use std::io;
use test_log::test;

struct TestServerHandler;

#[async_trait]
impl ServerHandler for TestServerHandler {
    type Request = u8;
    type Response = u8;
    type LocalData = ();

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        ctx.reply
            .send(ctx.request.payload)
            .await
            .expect("Failed to send response")
    }
}

#[test(tokio::test)]
async fn should_be_able_to_establish_a_single_connection_and_communicate() {
    let (t1, t2) = InmemoryTransport::pair(100);

    let mut config = Config::default();
    config.connect_handlers.insert(
        "scheme".to_string(),
        boxed_connect_handler!(|_a, _b, _c| {
            let (t1, t2) = InmemoryTransport::pair(100);

            // Spawn a server on one end and connect to it on the other
            let _ = Server::new()
                .handler(TestServerHandler)
                .verifier(Verifier::none())
                .start(OneshotListener::from_value(t2))?;

            let client = Client::build()
                .auth_handler(DummyAuthHandler)
                .reconnect_strategy(ReconnectStrategy::Fail)
                .connector(t1)
                .connect_untyped()
                .await?;

            Ok(client)
        }),
    );

    info!("Starting manager");
    let _manager_ref = ManagerServer::new(config)
        .verifier(Verifier::none())
        .start(OneshotListener::from_value(t2))
        .expect("Failed to start manager server");

    info!("Connecting to manager");
    let mut client: ManagerClient = Client::build()
        .auth_handler(DummyAuthHandler)
        .reconnect_strategy(ReconnectStrategy::Fail)
        .connector(t1)
        .connect()
        .await
        .expect("Failed to connect to manager");

    // Test establishing a connection to some remote server
    info!("Submitting server connection request to manager");
    let id = client
        .connect(
            "scheme://host".parse::<Destination>().unwrap(),
            "key=value".parse::<Map>().unwrap(),
            DummyAuthHandler,
        )
        .await
        .expect("Failed to connect to a remote server");

    // Test retrieving list of connections
    info!("Submitting connection list request to manager");
    let list = client
        .list()
        .await
        .expect("Failed to get list of connections");
    assert_eq!(list.len(), 1);
    assert_eq!(list.get(&id).unwrap().to_string(), "scheme://host");

    // Test retrieving information
    info!("Submitting connection info request to manager");
    let info = client
        .info(id)
        .await
        .expect("Failed to get info about connection");
    assert_eq!(info.id, id);
    assert_eq!(info.destination.to_string(), "scheme://host");
    assert_eq!(info.options, "key=value".parse::<Map>().unwrap());

    // Create a new channel and request some data
    info!("Submitting server channel open request to manager");
    let mut channel_client: Client<u8, u8> = client
        .open_raw_channel(id)
        .await
        .expect("Failed to open channel")
        .into_client();

    info!("Verifying server channel can send and receive data");
    let res = channel_client
        .send(123u8)
        .await
        .expect("Failed to send request to server");
    assert_eq!(res.payload, 123u8, "Invalid response payload");

    // Test killing a connection
    info!("Submitting connection kill request to manager");
    client.kill(id).await.expect("Failed to kill connection");

    // Test getting an error to ensure that serialization of that data works,
    // which we do by trying to access a connection that no longer exists
    info!("Verifying server connection held by manager has terminated");
    let err = client.info(id).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
}
