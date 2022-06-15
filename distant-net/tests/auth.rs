use distant_net::{
    AuthClient, AuthServer, AuthVerifyKind, Client, IntoSplit, MpscTransport, Question, ServerExt,
    TestListener,
};

/// Spawns a server and client connected together, returning the client
fn setup() -> AuthClient {
    // Make a pair of inmemory transports that we can use to test client and server connected
    let (t1, t2) = MpscTransport::pair(100);

    // Create the client
    let (writer, reader) = t1.into_split();
    let client = AuthClient::from(Client::new(writer, reader).unwrap());

    // Make a server that echos questions back as answers and only verifies the text "yes"
    let server = AuthServer {
        on_challenge: |questions, _| questions.into_iter().map(|x| x.text).collect(),
        on_verify: |_, text| text == "yes",
        on_info: |_| {},
        on_error: |_, _| {},
    };

    // Spawn the server to listen for our client to connect
    tokio::spawn(async move {
        let (writer, reader) = t2.into_split();
        let (tx, listener) = TestListener::channel(1);
        tx.send((writer, reader)).await.unwrap();
        let _server = server.start(listener).unwrap();
    });

    client
}

#[tokio::test]
async fn client_should_be_able_to_challenge_against_server() {
    let mut client = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // Now do the challenge
    assert_eq!(
        client
            .challenge(vec![Question::new("hello".to_string())], Default::default())
            .await
            .unwrap(),
        vec!["hello".to_string()]
    );
}

#[tokio::test]
async fn client_should_be_able_to_verify_against_server() {
    let mut client = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // "no" will yield false
    assert!(!client
        .verify(AuthVerifyKind::Host, "no".to_string())
        .await
        .unwrap());

    // "yes" will yield true
    assert!(client
        .verify(AuthVerifyKind::Host, "yes".to_string())
        .await
        .unwrap());
}
