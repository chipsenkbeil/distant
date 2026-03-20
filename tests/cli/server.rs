//! Integration tests for the `distant server listen` CLI subcommand.
//!
//! Verifies that the server starts up, prints credential URL to stdout,
//! and that the `--help` flag documents expected options.

use std::io::Read;
use std::process::Command;
use std::time::Duration;

use distant_test_harness::manager;
use distant_test_harness::process::TestChild;

#[test]
fn server_listen_should_output_credentials_and_exit() {
    let mut child = TestChild::spawn(Command::new(manager::bin_path()).args([
        "server",
        "listen",
        "--shutdown",
        "after=1",
    ]))
    .expect("Failed to spawn server");

    let mut stdout = child.stdout.take().unwrap();
    let mut output = String::new();
    let mut buf = [0u8; 4096];

    // Read with timeout - server should print credentials quickly
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                output.push_str(&String::from_utf8_lossy(&buf[..n]));
                if output.contains("distant://") {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }

    child.kill();

    assert!(
        output.contains("distant://"),
        "Expected credentials in output, got: {}",
        output
    );
}

#[test]
fn server_listen_custom_port() {
    let mut child = TestChild::spawn(Command::new(manager::bin_path()).args([
        "server",
        "listen",
        "--port",
        "0",
        "--shutdown",
        "after=1",
    ]))
    .expect("Failed to spawn server with --port 0");

    let mut stdout = child.stdout.take().unwrap();
    let mut output = String::new();
    let mut buf = [0u8; 4096];

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                output.push_str(&String::from_utf8_lossy(&buf[..n]));
                if output.contains("distant://") {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }

    child.kill();

    // The credentials URL should contain a port number (from the OS-assigned ephemeral port)
    assert!(
        output.contains("distant://"),
        "Expected credentials in output with custom port, got: {output}"
    );

    // Extract port from URL: distant://:key@host:PORT
    if let Some(url_start) = output.find("distant://") {
        let url = &output[url_start..];
        // Port appears after the last ':'
        if let Some(last_colon) = url.rfind(':') {
            let port_str: String = url[last_colon + 1..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let port: u16 = port_str.parse().expect("Expected valid port number");
            assert!(port > 0, "Expected non-zero port, got: {port}");
        }
    }
}

#[test]
fn server_listen_help_should_show_options() {
    let output = Command::new(manager::bin_path())
        .args(["server", "listen", "--help"])
        .output()
        .expect("Failed to run server listen --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--shutdown") || stdout.contains("shutdown"),
        "Expected --shutdown in help, got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}
