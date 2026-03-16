use std::io;

use distant_core::ChannelExt;
use rstest::*;
use test_log::test;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time;

use distant_test_harness::host::*;

#[rstest]
#[test(tokio::test)]
async fn tunnel_open_should_send_and_receive_data_then_auto_close_on_remote_end(
    #[future] ctx: ClientCtx,
) {
    let mut ctx = ctx.await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_port = listener.local_addr().unwrap().port();

    let mut tunnel = ctx
        .client
        .tunnel_open("127.0.0.1", target_port)
        .await
        .unwrap();
    let mut writer = tunnel.writer.take().unwrap();
    let mut reader = tunnel.reader.take().unwrap();

    let (mut stream, _) = listener.accept().await.unwrap();

    // Send data client -> remote
    let outgoing = b"Hello from client";
    writer.write(outgoing.to_vec()).await.unwrap();

    let mut buf = vec![0u8; 256];
    let n = stream.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], outgoing);

    // Send data remote -> client
    let incoming = b"Hello from remote";
    stream.write_all(incoming).await.unwrap();

    let data = reader.read().await.unwrap();
    assert_eq!(data, incoming.as_slice());

    // Remote closes its end
    drop(stream);
    drop(listener);

    // Tunnel should auto-close: reader returns BrokenPipe once the
    // server sends TunnelClosed and the incoming task exits
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(deadline, reader.read()).await {
            Ok(Ok(_data)) => continue,
            Ok(Err(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
                break;
            }
            Err(_) => panic!("Timed out waiting for tunnel to auto-close"),
        }
    }

    // Drop writer before wait() so the outgoing task can complete
    drop(writer);
    tunnel.wait().await;

    // Session must still be alive after tunnel closes
    let info = ctx.client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_open_should_fail_if_port_not_listening(#[future] ctx: ClientCtx) {
    let mut ctx = ctx.await;

    // Bind a listener to get an unused port, then drop it so nothing is listening
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unused_port = listener.local_addr().unwrap().port();
    drop(listener);

    let err = ctx
        .client
        .tunnel_open("127.0.0.1", unused_port)
        .await
        .expect_err("Expected error when connecting to closed port");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("refused") || err_msg.contains("connection"),
        "Expected connection error, got: {err_msg}",
    );
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_listen_should_relay_data_bidirectionally(#[future] ctx: ClientCtx) {
    let mut ctx = ctx.await;
    let timeout_dur = time::Duration::from_secs(10);

    let mut listener = ctx.client.tunnel_listen("127.0.0.1", 0).await.unwrap();
    let port = listener.port();

    let mut tcp_stream =
        time::timeout(timeout_dur, TcpStream::connect(format!("127.0.0.1:{port}")))
            .await
            .expect("Timed out connecting to listener")
            .unwrap();

    let incoming = time::timeout(timeout_dur, listener.next())
        .await
        .expect("Timed out waiting for incoming tunnel")
        .expect("Listener closed before receiving incoming tunnel");
    let mut writer = incoming.writer;
    let mut reader = incoming.reader;

    // TCP client -> tunnel reader
    let client_msg = b"Hello from TCP client";
    tcp_stream.write_all(client_msg).await.unwrap();

    let data = time::timeout(timeout_dur, reader.read())
        .await
        .expect("Timed out reading from tunnel reader")
        .unwrap();
    assert_eq!(data, client_msg.as_slice());

    // Tunnel writer -> TCP client
    let server_msg = b"Hello from tunnel writer";
    writer.write(server_msg.to_vec()).await.unwrap();

    let mut buf = vec![0u8; 256];
    let n = time::timeout(timeout_dur, tcp_stream.read(&mut buf))
        .await
        .expect("Timed out reading from TCP stream")
        .unwrap();
    assert_eq!(&buf[..n], server_msg);

    // Drop TCP client and verify reader gets BrokenPipe
    drop(tcp_stream);

    let deadline = time::Instant::now() + timeout_dur;
    loop {
        match time::timeout_at(deadline, reader.read()).await {
            Ok(Ok(_data)) => continue,
            Ok(Err(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
                break;
            }
            Err(_) => panic!("Timed out waiting for tunnel reader to report BrokenPipe"),
        }
    }

    // Clean up
    drop(writer);
    listener.close().await.unwrap();

    // Session must still be alive after listener closes
    let info = ctx.client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_listen_should_handle_multiple_connections(#[future] ctx: ClientCtx) {
    let mut ctx = ctx.await;
    let timeout_dur = time::Duration::from_secs(10);

    let mut listener = ctx.client.tunnel_listen("127.0.0.1", 0).await.unwrap();
    let port = listener.port();

    let tcp_stream_0 = time::timeout(timeout_dur, TcpStream::connect(format!("127.0.0.1:{port}")))
        .await
        .expect("Timed out connecting client 0")
        .unwrap();

    let incoming_0 = time::timeout(timeout_dur, listener.next())
        .await
        .expect("Timed out waiting for incoming tunnel 0")
        .expect("Listener closed before receiving tunnel 0");

    let tcp_stream_1 = time::timeout(timeout_dur, TcpStream::connect(format!("127.0.0.1:{port}")))
        .await
        .expect("Timed out connecting client 1")
        .unwrap();

    let incoming_1 = time::timeout(timeout_dur, listener.next())
        .await
        .expect("Timed out waiting for incoming tunnel 1")
        .expect("Listener closed before receiving tunnel 1");

    assert_ne!(
        incoming_0.tunnel_id, incoming_1.tunnel_id,
        "Sub-tunnels should have distinct IDs",
    );

    // Verify both tunnels are independently functional by sending distinct data
    // through each one
    let mut tcp_stream_0 = tcp_stream_0;
    let mut reader_0 = incoming_0.reader;
    tcp_stream_0.write_all(b"msg-for-tunnel-0").await.unwrap();
    let data_0 = time::timeout(timeout_dur, reader_0.read())
        .await
        .expect("Timed out reading from tunnel 0")
        .unwrap();
    assert_eq!(data_0, b"msg-for-tunnel-0");

    let mut tcp_stream_1 = tcp_stream_1;
    let mut reader_1 = incoming_1.reader;
    tcp_stream_1.write_all(b"msg-for-tunnel-1").await.unwrap();
    let data_1 = time::timeout(timeout_dur, reader_1.read())
        .await
        .expect("Timed out reading from tunnel 1")
        .unwrap();
    assert_eq!(data_1, b"msg-for-tunnel-1");

    // Clean up sub-tunnels before closing the listener
    drop(tcp_stream_0);
    drop(tcp_stream_1);
    drop(reader_0);
    drop(reader_1);
    drop(incoming_0.writer);
    drop(incoming_1.writer);

    // Allow server-side sub-tunnel tasks to observe the TCP disconnects and
    // send their cleanup messages before we close the listener
    tokio::task::yield_now().await;

    listener.close().await.unwrap();

    // Session must still be alive after multi-connection teardown
    let info = ctx.client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_listen_should_close_sub_tunnel_when_remote_disconnects(#[future] ctx: ClientCtx) {
    let mut ctx = ctx.await;
    let timeout_dur = time::Duration::from_secs(10);

    let mut listener = ctx.client.tunnel_listen("127.0.0.1", 0).await.unwrap();
    let port = listener.port();

    let tcp_stream = time::timeout(timeout_dur, TcpStream::connect(format!("127.0.0.1:{port}")))
        .await
        .expect("Timed out connecting to listener")
        .unwrap();

    let incoming = time::timeout(timeout_dur, listener.next())
        .await
        .expect("Timed out waiting for incoming tunnel")
        .expect("Listener closed before receiving incoming tunnel");
    let mut writer = incoming.writer;
    let mut reader = incoming.reader;

    // Drop the TCP client to simulate remote disconnect
    drop(tcp_stream);

    // Reader should eventually return BrokenPipe
    let deadline = time::Instant::now() + timeout_dur;
    loop {
        match time::timeout_at(deadline, reader.read()).await {
            Ok(Ok(_data)) => continue,
            Ok(Err(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
                break;
            }
            Err(_) => panic!("Timed out waiting for reader to report BrokenPipe after disconnect"),
        }
    }

    // Writer should also be closed (the sub-tunnel's server side has been cleaned up)
    let write_result = time::timeout(timeout_dur, writer.write(b"should fail".to_vec())).await;
    match write_result {
        Ok(Err(e)) => {
            assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        }
        Ok(Ok(())) => {
            // First write after close may succeed if it's buffered; subsequent
            // reads will still fail. This is acceptable.
        }
        Err(_) => panic!("Timed out waiting for writer to fail"),
    }

    // Clean up
    drop(writer);
    drop(reader);
    listener.close().await.unwrap();

    // Session must still be alive
    let info = ctx.client.system_info().await.unwrap();
    assert_eq!(info.family, std::env::consts::FAMILY);
}

#[rstest]
#[test(tokio::test)]
async fn tunnel_listen_should_fail_if_port_already_in_use(#[future] ctx: ClientCtx) {
    let mut ctx = ctx.await;

    // Bind a local listener to occupy a port
    let blocker = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied_port = blocker.local_addr().unwrap().port();

    // Ask the remote to listen on the same port — should fail
    let err = ctx
        .client
        .tunnel_listen("127.0.0.1", occupied_port)
        .await
        .expect_err("Expected error when binding to occupied port");
    let err_msg = err.to_string().to_lowercase();
    assert!(
        err_msg.contains("address") || err_msg.contains("in use") || err_msg.contains("bind"),
        "Expected address-in-use error, got: {err_msg}",
    );
}
