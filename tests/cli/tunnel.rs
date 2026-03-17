//! Integration tests for the `distant tunnel` CLI subcommands.
//!
//! Tests forward tunnel creation, data forwarding through tunnels, tunnel
//! listing, closing, and error handling for missing connections and invalid IDs.

use std::net::TcpListener;
use std::process::Stdio;
use std::time::Duration;

use rstest::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::time;

use distant_test_harness::backend::{Backend, ctx_for_backend};
use distant_test_harness::exe;
use distant_test_harness::manager::*;
use distant_test_harness::skip_if_no_backend;

/// Parses the tunnel ID and actual port from a "Tunnel N started: ..." output line.
///
/// For forward tunnels the format is:
///   `Tunnel 1 started: 127.0.0.1:54321 -> 127.0.0.1:9999`
///
/// Returns `(tunnel_id, bound_port)`.
fn parse_tunnel_started(output: &str) -> (u32, u16) {
    let line = output
        .lines()
        .find(|l| l.contains("Tunnel") && l.contains("started:"))
        .unwrap_or_else(|| panic!("no 'Tunnel ... started:' line in output:\n{output}"));

    let words: Vec<&str> = line.split_whitespace().collect();

    // words: ["Tunnel", "<id>", "started:", "127.0.0.1:<port>", "->", ...]
    let id: u32 = words[1]
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse tunnel ID from '{line}': {e}"));

    let addr_part = words[3]; // "127.0.0.1:<port>" or "remote"
    let port: u16 = if addr_part == "remote" {
        // Reverse tunnel format: "Tunnel N started: remote port <port> -> ..."
        // words: ["Tunnel", "<id>", "started:", "remote", "port", "<port>", "->", ...]
        words[5]
            .parse()
            .unwrap_or_else(|e| panic!("failed to parse remote port from '{line}': {e}"))
    } else {
        // Forward tunnel format: "Tunnel N started: 127.0.0.1:<port> -> ..."
        addr_part
            .rsplit(':')
            .next()
            .unwrap_or_else(|| panic!("no colon in address part '{addr_part}'"))
            .parse()
            .unwrap_or_else(|e| panic!("failed to parse port from '{addr_part}': {e}"))
    };

    (id, port)
}

#[tokio::test]
async fn tunnel_open_forwards_tcp_data() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        output.status.success(),
        "tunnel open should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, local_port) = parse_tunnel_started(&stdout_str);

    assert!(local_port > 0, "tunnel should bind to a real port");

    let mut stream = time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{local_port}")),
    )
    .await
    .expect("timed out connecting to tunnel")
    .expect("failed to connect to tunnel");

    let payload = b"hello tunnel";
    stream
        .write_all(payload)
        .await
        .expect("failed to write to tunnel");

    // Allow the data to propagate through the tunnel before signaling EOF
    time::sleep(Duration::from_millis(200)).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through tunnel")
        .expect("failed to read response through tunnel");

    assert_eq!(
        response, payload,
        "data through tunnel should match what was sent"
    );
}

#[tokio::test]
async fn tunnel_open_prints_local_port() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("15")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        output.status.success(),
        "tunnel open should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.contains("Tunnel") && stdout_str.contains("started:"),
        "output should contain tunnel started message, got: {stdout_str}"
    );

    let (id, port) = parse_tunnel_started(&stdout_str);
    assert!(id > 0, "tunnel ID should be positive");
    assert!(port > 0, "tunnel port should be positive");
    assert!(
        stdout_str.contains(&format!("127.0.0.1:{port}")),
        "output should contain the bound address, got: {stdout_str}"
    );
}

#[rstest]
#[test_log::test]
fn tunnel_open_no_connection(manager_only_ctx: ManagerOnlyCtx) {
    // Manager is running but no connection has been established, so tunnel open should fail
    let output = manager_only_ctx
        .new_std_cmd(["tunnel", "open"])
        .arg("0:127.0.0.1:9999")
        .output()
        .expect("failed to run tunnel open");

    assert!(
        !output.status.success(),
        "tunnel open without a connection should fail"
    );
}

#[rstest]
#[test_log::test]
fn tunnel_list_empty(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(["tunnel", "list"])
        .output()
        .expect("failed to run tunnel list");

    assert!(
        output.status.success(),
        "tunnel list should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.contains("No active tunnels"),
        "expected 'No active tunnels' in output, got: {stdout_str}"
    );
}

#[tokio::test]
async fn tunnel_list_shows_active_tunnels() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("15")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    // Open a tunnel
    let spec = format!("0:127.0.0.1:{echo_port}");
    let open_output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        open_output.status.success(),
        "tunnel open should succeed, stderr: {}",
        String::from_utf8_lossy(&open_output.stderr)
    );

    // List tunnels
    let list_output = ctx
        .new_std_cmd(["tunnel", "list"])
        .output()
        .expect("failed to run tunnel list");

    assert!(
        list_output.status.success(),
        "tunnel list should succeed, stderr: {}",
        String::from_utf8_lossy(&list_output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        stdout_str.contains("forward"),
        "tunnel list should show a forward tunnel, got: {stdout_str}"
    );
    assert!(
        stdout_str.contains("127.0.0.1"),
        "tunnel list should show the remote host, got: {stdout_str}"
    );
    assert!(
        !stdout_str.contains("No active tunnels"),
        "tunnel list should NOT say 'No active tunnels' after opening one, got: {stdout_str}"
    );
}

#[tokio::test]
async fn tunnel_close_by_id() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("15")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    // Open a tunnel
    let spec = format!("0:127.0.0.1:{echo_port}");
    let open_output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        open_output.status.success(),
        "tunnel open should succeed, stderr: {}",
        String::from_utf8_lossy(&open_output.stderr)
    );

    let open_stdout = String::from_utf8_lossy(&open_output.stdout);
    let (id, _port) = parse_tunnel_started(&open_stdout);

    // Close the tunnel by ID
    let close_output = ctx
        .new_std_cmd(["tunnel", "close"])
        .arg(id.to_string())
        .output()
        .expect("failed to run tunnel close");

    assert!(
        close_output.status.success(),
        "tunnel close should succeed, stderr: {}",
        String::from_utf8_lossy(&close_output.stderr)
    );

    let close_stdout = String::from_utf8_lossy(&close_output.stdout);
    let expected_msg = format!("Tunnel {id} closed");
    assert!(
        close_stdout.contains(&expected_msg),
        "expected '{expected_msg}' in output, got: {close_stdout}"
    );
}

#[rstest]
#[test_log::test]
fn tunnel_close_invalid_id(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(["tunnel", "close"])
        .arg("99999")
        .output()
        .expect("failed to run tunnel close");

    assert!(
        !output.status.success(),
        "tunnel close with invalid ID should fail"
    );
}

#[tokio::test]
async fn tunnel_open_specific_local_port() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("15")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    // Find a free port by binding to port 0, recording the assigned port, then dropping
    // the listener so the tunnel can bind to it.
    let specific_port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind to ephemeral port");
        let port = listener.local_addr().expect("no local addr").port();
        drop(listener);
        port
    };

    let spec = format!("{specific_port}:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        output.status.success(),
        "tunnel open with specific port should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, bound_port) = parse_tunnel_started(&stdout_str);

    assert_eq!(
        bound_port, specific_port,
        "tunnel should bind to the requested port {specific_port}, got {bound_port}"
    );
}

#[tokio::test]
async fn tunnel_open_invalid_address() {
    let ctx = ManagerCtx::start();

    // 192.0.2.1 is TEST-NET-1 (RFC 5737), unreachable by design.
    // The tunnel is created lazily — it binds locally but only connects to the remote
    // on first data, so `tunnel open` itself should succeed.
    let spec = "0:192.0.2.1:99";
    let output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(spec)
        .output()
        .expect("failed to run tunnel open");

    if output.status.success() {
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout_str.contains("Tunnel") && stdout_str.contains("started:"),
            "successful tunnel open should print tunnel started message, got: {stdout_str}"
        );

        let (id, port) = parse_tunnel_started(&stdout_str);
        assert!(id > 0, "tunnel ID should be positive");
        assert!(port > 0, "tunnel port should be positive");
    } else {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr_str.is_empty(),
            "failed tunnel open should produce an error message"
        );
    }
}

#[tokio::test]
async fn tunnel_listen_forwards_tcp_data() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    // Start the echo server locally — the reverse tunnel will forward remote traffic to it
    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    // Open a reverse tunnel: remote listens on ephemeral port, forwards to local echo server
    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "listen"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel listen");

    assert!(
        output.status.success(),
        "tunnel listen should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, remote_port) = parse_tunnel_started(&stdout_str);

    assert!(remote_port > 0, "reverse tunnel should bind to a real port");

    // Since the server is local (Host backend), the remote port is on localhost too.
    // Connect to the remote port and verify data flows through to the echo server.
    let mut stream = time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{remote_port}")),
    )
    .await
    .expect("timed out connecting to reverse tunnel")
    .expect("failed to connect to reverse tunnel");

    let payload = b"hello reverse tunnel";
    stream
        .write_all(payload)
        .await
        .expect("failed to write to reverse tunnel");

    // Allow the data to propagate through the tunnel before signaling EOF
    time::sleep(Duration::from_millis(200)).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through reverse tunnel")
        .expect("failed to read response through reverse tunnel");

    assert_eq!(
        response, payload,
        "data through reverse tunnel should match what was sent"
    );
}

#[tokio::test]
async fn tunnel_listen_prints_remote_port() {
    let ctx = ManagerCtx::start();

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("15")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "listen"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel listen");

    assert!(
        output.status.success(),
        "tunnel listen should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.contains("Tunnel") && stdout_str.contains("started:"),
        "output should contain tunnel started message, got: {stdout_str}"
    );
    assert!(
        stdout_str.contains("remote port"),
        "reverse tunnel output should contain 'remote port', got: {stdout_str}"
    );

    let (id, port) = parse_tunnel_started(&stdout_str);
    assert!(id > 0, "tunnel ID should be positive");
    assert!(port > 0, "tunnel port should be positive");
}

#[rstest]
#[test_log::test]
fn tunnel_listen_no_connection(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["tunnel", "listen"])
        .arg("0:127.0.0.1:9999")
        .output()
        .expect("failed to run tunnel listen");

    assert!(
        !output.status.success(),
        "tunnel listen without a connection should fail"
    );
}

#[rstest]
#[test_log::test]
fn tunnel_listen_invalid_address(ctx: ManagerCtx) {
    // Attempt to listen on a privileged port (port 1) which should fail
    let output = ctx
        .new_std_cmd(["tunnel", "listen"])
        .arg("1:127.0.0.1:9999")
        .output()
        .expect("failed to run tunnel listen");

    // Binding to port 1 should fail (privileged port, or at least unlikely to succeed)
    // If it somehow succeeds (e.g., running as root), just verify the command didn't crash
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.is_empty(),
            "failed tunnel listen should produce error output"
        );
    }
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[tokio::test]
async fn tunnel_open_data_cross_backend(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        output.status.success(),
        "tunnel open should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, local_port) = parse_tunnel_started(&stdout_str);
    assert!(local_port > 0, "tunnel should bind to a real port");

    let mut stream = time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{local_port}")),
    )
    .await
    .expect("timed out connecting to tunnel")
    .expect("failed to connect to tunnel");

    let payload = b"cross-backend tunnel data";
    stream
        .write_all(payload)
        .await
        .expect("failed to write to tunnel");

    time::sleep(Duration::from_millis(200)).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through tunnel")
        .expect("failed to read response through tunnel");

    assert_eq!(
        response, payload,
        "data through tunnel should match what was sent via {backend:?}"
    );
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[tokio::test]
async fn tunnel_listen_data_cross_backend(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));

    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = tokio::process::Command::new(&echo_bin)
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(Duration::from_secs(10), reader.read_line(&mut port_line))
        .await
        .expect("timed out waiting for echo server port")
        .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    let spec = format!("0:127.0.0.1:{echo_port}");
    let output = ctx
        .new_std_cmd(["tunnel", "listen"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel listen");

    assert!(
        output.status.success(),
        "tunnel listen should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, remote_port) = parse_tunnel_started(&stdout_str);
    assert!(remote_port > 0, "reverse tunnel should bind to a real port");

    let mut stream = time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(format!("127.0.0.1:{remote_port}")),
    )
    .await
    .expect("timed out connecting to reverse tunnel")
    .expect("failed to connect to reverse tunnel");

    let payload = b"cross-backend reverse tunnel data";
    stream
        .write_all(payload)
        .await
        .expect("failed to write to reverse tunnel");

    time::sleep(Duration::from_millis(200)).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through reverse tunnel")
        .expect("failed to read response through reverse tunnel");

    assert_eq!(
        response, payload,
        "data through reverse tunnel should match via {backend:?}"
    );
}
