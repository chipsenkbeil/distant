use distant_net::{
    AuthClient, AuthErrorKind, AuthQuestion, AuthRequest, AuthServer, AuthVerifyKind, Client,
    IntoSplit, MpscListener, MpscTransport, ServerExt,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Spawns a server and client connected together, returning the client
fn setup() -> (AuthClient, mpsc::Receiver<AuthRequest>) {
    // Make a pair of inmemory transports that we can use to test client and server connected
    let (t1, t2) = MpscTransport::pair(100);

    // Create the client
    let (writer, reader) = t1.into_split();
    let client = AuthClient::from(Client::new(writer, reader).unwrap());

    // Prepare a channel where we can pass back out whatever request we get
    let (tx, rx) = mpsc::channel(100);

    let tx_2 = tx.clone();
    let tx_3 = tx.clone();
    let tx_4 = tx.clone();

    // Make a server that echos questions back as answers and only verifies the text "yes"
    let server = AuthServer {
        on_challenge: move |questions, extra| {
            let questions_2 = questions.clone();
            tx.try_send(AuthRequest::Challenge { questions, extra })
                .unwrap();
            questions_2.into_iter().map(|x| x.text).collect()
        },
        on_verify: move |kind, text| {
            let valid = text == "yes";
            tx_2.try_send(AuthRequest::Verify { kind, text }).unwrap();
            valid
        },
        on_info: move |text| {
            tx_3.try_send(AuthRequest::Info { text }).unwrap();
        },
        on_error: move |kind, text| {
            tx_4.try_send(AuthRequest::Error { kind, text }).unwrap();
        },
    };

    // Spawn the server to listen for our client to connect
    tokio::spawn(async move {
        let (writer, reader) = t2.into_split();
        let (tx, listener) = MpscListener::channel(1);
        tx.send((writer, reader)).await.unwrap();
        let _server = server.start(listener).unwrap();
    });

    (client, rx)
}

#[tokio::test]
async fn client_should_be_able_to_challenge_against_server() {
    let (mut client, mut rx) = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // Now do the challenge
    assert_eq!(
        client
            .challenge(
                vec![AuthQuestion::new("hello".to_string())],
                Default::default()
            )
            .await
            .unwrap(),
        vec!["hello".to_string()]
    );

    // Verify that the server received the request
    let request = rx.recv().await.unwrap();
    match request {
        AuthRequest::Challenge { questions, extra } => {
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].text, "hello");
            assert_eq!(questions[0].extra, HashMap::new());

            assert_eq!(extra, HashMap::new());
        }
        x => panic!("Unexpected request received by server: {:?}", x),
    }
}

#[tokio::test]
async fn client_should_be_able_to_verify_against_server() {
    let (mut client, mut rx) = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // "no" will yield false
    assert!(!client
        .verify(AuthVerifyKind::Host, "no".to_string())
        .await
        .unwrap());

    // Verify that the server received the request
    let request = rx.recv().await.unwrap();
    match request {
        AuthRequest::Verify { kind, text } => {
            assert_eq!(kind, AuthVerifyKind::Host);
            assert_eq!(text, "no");
        }
        x => panic!("Unexpected request received by server: {:?}", x),
    }

    // "yes" will yield true
    assert!(client
        .verify(AuthVerifyKind::Host, "yes".to_string())
        .await
        .unwrap());

    // Verify that the server received the request
    let request = rx.recv().await.unwrap();
    match request {
        AuthRequest::Verify { kind, text } => {
            assert_eq!(kind, AuthVerifyKind::Host);
            assert_eq!(text, "yes");
        }
        x => panic!("Unexpected request received by server: {:?}", x),
    }
}

#[tokio::test]
async fn client_should_be_able_to_send_info_to_server() {
    let (mut client, mut rx) = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // Send some information
    client.info(String::from("hello, world")).await.unwrap();

    // Verify that the server received the request
    let request = rx.recv().await.unwrap();
    match request {
        AuthRequest::Info { text } => assert_eq!(text, "hello, world"),
        x => panic!("Unexpected request received by server: {:?}", x),
    }
}

#[tokio::test]
async fn client_should_be_able_to_send_error_to_server() {
    let (mut client, mut rx) = setup();

    // Gotta start with the handshake first
    client.handshake().await.unwrap();

    // Send some error
    client
        .error(AuthErrorKind::Unknown, String::from("hello, world"))
        .await
        .unwrap();

    // Verify that the server received the request
    let request = rx.recv().await.unwrap();
    match request {
        AuthRequest::Error { kind, text } => {
            assert_eq!(kind, AuthErrorKind::Unknown);
            assert_eq!(text, "hello, world");
        }
        x => panic!("Unexpected request received by server: {:?}", x),
    }
}
