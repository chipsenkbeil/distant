use async_trait::async_trait;
use distant_net::client::Client;
use distant_net::common::authentication::{DummyAuthHandler, Verifier};
use distant_net::common::{InmemoryTransport, OneshotListener, Request};
use distant_net::server::{Server, ServerCtx, ServerHandler};
use log::*;
use test_log::test;

struct TestServerHandler;

#[async_trait]
impl ServerHandler for TestServerHandler {
    type Request = (u8, String);
    type Response = String;
    type LocalData = ();

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let (cnt, msg) = ctx.request.payload;

        for i in 0..cnt {
            ctx.reply
                .send(format!("echo {i} {msg}"))
                .await
                .expect("Failed to send response");
        }
    }
}

#[test(tokio::test)]
async fn should_be_able_to_send_and_receive_untyped_payloads_between_client_and_server() {
    let (t1, t2) = InmemoryTransport::pair(100);

    let _ = Server::new()
        .handler(TestServerHandler)
        .verifier(Verifier::none())
        .start(OneshotListener::from_value(t2))
        .expect("Failed to start server");

    let mut client = Client::build()
        .auth_handler(DummyAuthHandler)
        .connector(t1)
        .connect_untyped()
        .await
        .expect("Failed to connect to server");

    info!("Mailing a message from the client, and waiting for 3 responses");
    let mut mailbox = client
        .mail(
            Request::new((3, "hello".to_string()))
                .to_untyped_request()
                .unwrap(),
        )
        .await
        .expect("Failed to mail message");

    assert_eq!(
        mailbox
            .next()
            .await
            .unwrap()
            .to_typed_response::<String>()
            .unwrap()
            .payload,
        "echo 0 hello"
    );
    assert_eq!(
        mailbox
            .next()
            .await
            .unwrap()
            .to_typed_response::<String>()
            .unwrap()
            .payload,
        "echo 1 hello"
    );
    assert_eq!(
        mailbox
            .next()
            .await
            .unwrap()
            .to_typed_response::<String>()
            .unwrap()
            .payload,
        "echo 2 hello"
    );

    info!("Sending a message from the client, and waiting for a response");
    let response = client
        .send(
            Request::new((1, "hello".to_string()))
                .to_untyped_request()
                .unwrap(),
        )
        .await
        .expect("Failed to send message");

    assert_eq!(
        response.to_typed_response::<String>().unwrap().payload,
        "echo 0 hello"
    );

    info!("Firing off a message from the client");
    client
        .fire(
            Request::new((1, "hello".to_string()))
                .to_untyped_request()
                .unwrap(),
        )
        .await
        .expect("Failed to fire message");
}
