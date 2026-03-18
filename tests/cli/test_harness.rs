//! Smoke tests for `distant-test-harness` helper binaries.
//!
//! These tests verify the test infrastructure itself rather than the distant
//! CLI. Currently covers the `tcp-echo-server` binary.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::time;

use distant_test_harness::exe;

#[tokio::test]
async fn tcp_echo_server_roundtrips_data() {
    let bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut child = tokio::process::Command::new(&bin)
        .arg("10")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for port on stdout")
        .expect("failed to read port from stdout");

    let port: u16 = port_line
        .trim()
        .parse()
        .expect("first line of stdout should be a port number");

    let mut stream = time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")),
    )
    .await
    .expect("timed out connecting to echo server")
    .expect("failed to connect to echo server");

    let payload = b"hello world";

    stream
        .write_all(payload)
        .await
        .expect("failed to write to echo server");
    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response from echo server")
        .expect("failed to read response from echo server");

    assert_eq!(
        response, payload,
        "echo server should return the same data that was sent"
    );
}
