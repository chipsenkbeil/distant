use distant_core::{InmemoryStream, SecretKey, Transport, TransportError};
use std::{io, sync::Arc};

const BUFFER_SIZE: usize = 100;

#[tokio::test]
async fn transport_should_be_able_to_send_encrypted_data_to_other_side_to_decrypt() {
    let (src, dst) = InmemoryStream::pair(BUFFER_SIZE);

    // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
    let (src, dst) = tokio::join!(
        Transport::from_handshake(src, None),
        Transport::from_handshake(dst, None)
    );

    let mut src = src.expect("src stream failed handshake");
    let mut dst = dst.expect("dst stream failed handshake");

    src.send("some data").await.expect("Failed to send data");
    let data = dst
        .receive::<String>()
        .await
        .expect("Failed to receive data")
        .expect("Data missing");

    assert_eq!(data, "some data");
}

#[tokio::test]
async fn transport_should_be_able_to_sign_and_validate_signature_if_auth_key_included() {
    let (src, dst) = InmemoryStream::pair(BUFFER_SIZE);

    let auth_key = Arc::new(SecretKey::default());

    // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
    let (src, dst) = tokio::join!(
        Transport::from_handshake(src, Some(Arc::clone(&auth_key))),
        Transport::from_handshake(dst, Some(auth_key))
    );

    let mut src = src.expect("src stream failed handshake");
    let mut dst = dst.expect("dst stream failed handshake");

    src.send("some data").await.expect("Failed to send data");
    let data = dst
        .receive::<String>()
        .await
        .expect("Failed to receive data")
        .expect("Data missing");

    assert_eq!(data, "some data");
}

#[tokio::test]
async fn transport_receive_should_fail_if_auth_key_differs_from_other_end() {
    let (src, dst) = InmemoryStream::pair(BUFFER_SIZE);

    // Make two transports with different auth keys
    // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
    let (src, dst) = tokio::join!(
        Transport::from_handshake(src, Some(Arc::new(SecretKey::default()))),
        Transport::from_handshake(dst, Some(Arc::new(SecretKey::default())))
    );

    let mut src = src.expect("src stream failed handshake");
    let mut dst = dst.expect("dst stream failed handshake");

    src.send("some data").await.expect("Failed to send data");
    match dst.receive::<String>().await {
        Err(TransportError::AuthError(_)) => {}
        x => panic!("Unexpected result: {:?}", x),
    }
}

#[tokio::test]
async fn transport_receive_should_fail_if_has_auth_key_while_sender_did_not_use_one() {
    let (src, dst) = InmemoryStream::pair(BUFFER_SIZE);

    // Make two transports with different auth keys
    // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
    let (src, dst) = tokio::join!(
        Transport::from_handshake(dst, None),
        Transport::from_handshake(src, Some(Arc::new(SecretKey::default())))
    );

    let mut src = src.expect("src stream failed handshake");
    let mut dst = dst.expect("dst stream failed handshake");

    src.send("some data").await.expect("Failed to send data");

    // NOTE: This keeps going between auth and io error about tag length because of the
    //       random data generated that can cause a different length to be perceived; so,
    //       we have to check for both
    match dst.receive::<String>().await {
        Err(TransportError::AuthError(_)) => {}
        Err(TransportError::IoError(x)) if matches!(x.kind(), io::ErrorKind::InvalidData) => {}
        x => panic!("Unexpected result: {:?}", x),
    }
}

#[tokio::test]
async fn transport_receive_should_fail_if_has_no_auth_key_while_sender_used_one() {
    let (src, dst) = InmemoryStream::pair(BUFFER_SIZE);

    // Make two transports with different auth keys
    // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
    let (src, dst) = tokio::join!(
        Transport::from_handshake(src, Some(Arc::new(SecretKey::default()))),
        Transport::from_handshake(dst, None)
    );

    let mut src = src.expect("src stream failed handshake");
    let mut dst = dst.expect("dst stream failed handshake");

    src.send("some data").await.expect("Failed to send data");
    match dst.receive::<String>().await {
        Err(TransportError::EncryptError(_)) => {}
        x => panic!("Unexpected result: {:?}", x),
    }
}
