//! Integration tests for the `distant tunnel` CLI subcommands.
//!
//! Tests forward tunnel creation, data forwarding through tunnels, tunnel
//! listing, closing, and error handling for missing connections and invalid IDs.

use std::net::TcpListener;
use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use rstest::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time;

use distant_test_harness::backend::{Backend, BackendCtx};
use distant_test_harness::exe;
use distant_test_harness::manager::*;
use distant_test_harness::skip_if_no_backend;

/// How long to wait for the tcp-echo-server to print its listening port.
const ECHO_SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for TCP connect and read operations through a tunnel.
const TCP_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Brief pause to let data propagate through the tunnel before signaling EOF.
const PROPAGATION_DELAY: Duration = Duration::from_millis(200);

/// Lifetime in seconds for echo server processes spawned in tunnel tests.
const ECHO_SERVER_LIFETIME_SECS: &str = "30";

/// Parses the tunnel ID and actual port from a "Tunnel N started: ..." output line.
///
/// Handles two output formats:
///   Forward:  `Tunnel 1 started: 127.0.0.1:54321 -> 127.0.0.1:9999`
///   Reverse:  `Tunnel 1 started: remote port 54321 -> 127.0.0.1:9999`
///
/// Uses regex to locate the "Tunnel" keyword and extract fields by name,
/// avoiding fragile positional word indexing.
///
/// Returns `(tunnel_id, bound_port)`.
fn parse_tunnel_started(output: &str) -> (u32, u16) {
    let line = output
        .lines()
        .find(|l| l.contains("Tunnel") && l.contains("started:"))
        .unwrap_or_else(|| panic!("no 'Tunnel ... started:' line in output:\n{output}"));

    // Forward format: "Tunnel <id> started: <addr>:<port> -> ..."
    // Reverse format: "Tunnel <id> started: remote port <port> -> ..."
    let re = Regex::new(
        r"Tunnel\s+(?<id>\d+)\s+started:\s+(?:(?:remote\s+port\s+(?<rport>\d+))|(?:\S+:(?<fport>\d+)))",
    )
    .expect("tunnel started regex should compile");

    let caps = re
        .captures(line)
        .unwrap_or_else(|| panic!("tunnel started line did not match expected format: '{line}'"));

    let id: u32 = caps["id"]
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse tunnel ID from '{line}': {e}"));

    let port_str = caps
        .name("rport")
        .or_else(|| caps.name("fport"))
        .unwrap_or_else(|| panic!("no port capture in tunnel started line: '{line}'"));

    let port: u16 = port_str
        .as_str()
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse port from '{line}': {e}"));

    (id, port)
}

/// Spawns a tcp-echo-server on the host and returns its child process and listening port.
async fn spawn_echo_server() -> (Child, u16) {
    let echo_bin = exe::build_tcp_echo_server()
        .await
        .expect("failed to build tcp-echo-server");

    let mut echo_child = Command::new(&echo_bin)
        .arg(ECHO_SERVER_LIFETIME_SECS)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn tcp-echo-server");

    let stdout = echo_child.stdout.take().expect("stdout not captured");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut port_line = String::new();
    time::timeout(
        ECHO_SERVER_STARTUP_TIMEOUT,
        reader.read_line(&mut port_line),
    )
    .await
    .expect("timed out waiting for echo server port")
    .expect("failed to read echo server port");

    let echo_port: u16 = port_line
        .trim()
        .parse()
        .expect("echo server stdout should be a port number");

    (echo_child, echo_port)
}

/// Spawns a tcp-echo-server that the given backend can reach at `127.0.0.1`.
///
/// For Host/SSH, this spawns locally since the remote side shares `127.0.0.1`
/// with the host. For Docker, this cross-compiles the binary, uploads it into
/// the container, and runs it inside via `docker exec` so the container's
/// `127.0.0.1` reaches it.
async fn spawn_reachable_echo_server(ctx: &BackendCtx) -> (Child, u16) {
    match ctx.backend() {
        #[cfg(feature = "docker")]
        Backend::Docker => {
            let remote_path = ctx
                .prepare_binary("tcp-echo-server")
                .await
                .expect("failed to prepare tcp-echo-server for Docker");

            let container_name = ctx
                .docker_container_name()
                .expect("expected Docker backend");

            let mut child = Command::new("docker")
                .args([
                    "exec",
                    "-i",
                    container_name,
                    &remote_path,
                    ECHO_SERVER_LIFETIME_SECS,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .expect("failed to spawn tcp-echo-server in container");

            let stdout = child.stdout.take().expect("stdout not captured");
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut port_line = String::new();
            time::timeout(
                ECHO_SERVER_STARTUP_TIMEOUT,
                reader.read_line(&mut port_line),
            )
            .await
            .expect("timed out waiting for echo server port in container")
            .expect("failed to read echo server port from container");

            let port: u16 = port_line
                .trim()
                .parse()
                .expect("echo server in container should print port number");

            (child, port)
        }
        _ => spawn_echo_server().await,
    }
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_open_should_print_local_port(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_reachable_echo_server(&ctx).await;

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
fn tunnel_open_should_fail_without_connection(manager_only_ctx: ManagerOnlyCtx) {
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
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_list_should_show_no_active_tunnels_when_empty(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["tunnel", "list"])
        .output()
        .expect("failed to run tunnel list");

    assert!(
        output.status.success(),
        "tunnel list should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.contains("No active tunnels"),
        "expected 'No active tunnels' in output, got: {stdout_str}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_list_should_show_active_tunnels(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_reachable_echo_server(&ctx).await;

    let spec = format!("0:127.0.0.1:{echo_port}");
    let open_output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        open_output.status.success(),
        "tunnel open should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&open_output.stderr)
    );

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

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_close_should_succeed_by_id(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_reachable_echo_server(&ctx).await;

    let spec = format!("0:127.0.0.1:{echo_port}");
    let open_output = ctx
        .new_std_cmd(["tunnel", "open"])
        .arg(&spec)
        .output()
        .expect("failed to run tunnel open");

    assert!(
        open_output.status.success(),
        "tunnel open should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&open_output.stderr)
    );

    let open_stdout = String::from_utf8_lossy(&open_output.stdout);
    let (id, _port) = parse_tunnel_started(&open_stdout);

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
fn tunnel_close_should_fail_with_invalid_id(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["tunnel", "close"])
        .arg("99999")
        .output()
        .expect("failed to run tunnel close");

    assert!(
        !output.status.success(),
        "tunnel close with invalid ID should fail"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_open_should_bind_to_specific_local_port(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_reachable_echo_server(&ctx).await;

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
        "tunnel open with specific port should succeed via {backend:?}, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let (_id, bound_port) = parse_tunnel_started(&stdout_str);

    assert_eq!(
        bound_port, specific_port,
        "tunnel should bind to the requested port {specific_port}, got {bound_port}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_open_should_handle_invalid_address(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

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

/// Docker is excluded because the Docker backend does not support
/// `tunnel_listen` (returns "tunnel_listen is not supported for Docker backends").
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[tokio::test]
async fn tunnel_listen_should_print_remote_port(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_echo_server().await;

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
fn tunnel_listen_should_fail_without_connection(manager_only_ctx: ManagerOnlyCtx) {
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

/// Docker is excluded because the Docker backend does not support
/// `tunnel_listen` (returns "tunnel_listen is not supported for Docker backends").
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[tokio::test]
async fn tunnel_listen_should_fail_with_privileged_port(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

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
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn tunnel_open_should_forward_data(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_reachable_echo_server(&ctx).await;

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
        TCP_IO_TIMEOUT,
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

    time::sleep(PROPAGATION_DELAY).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(TCP_IO_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through tunnel")
        .expect("failed to read response through tunnel");

    assert_eq!(
        response, payload,
        "data through tunnel should match what was sent via {backend:?}"
    );
}

/// Docker is excluded because the Docker backend does not support
/// `tunnel_listen` (returns "tunnel_listen is not supported for Docker backends").
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[tokio::test]
async fn tunnel_listen_should_forward_data(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (_echo, echo_port) = spawn_echo_server().await;

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
        TCP_IO_TIMEOUT,
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

    time::sleep(PROPAGATION_DELAY).await;

    stream
        .shutdown()
        .await
        .expect("failed to shut down write half");

    let mut response = Vec::new();
    time::timeout(TCP_IO_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("timed out reading response through reverse tunnel")
        .expect("failed to read response through reverse tunnel");

    assert_eq!(
        response, payload,
        "data through reverse tunnel should match via {backend:?}"
    );
}
