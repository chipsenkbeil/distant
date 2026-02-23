use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

fn bin_path() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_distant")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| which::which("distant").expect("distant binary not found"))
}

#[test]
fn server_listen_should_output_credentials_and_exit() {
    let mut child = Command::new(bin_path())
        .args(["server", "listen", "--shutdown", "after=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
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
                // Credentials contain "distant://"
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

    // Kill the server
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        output.contains("distant://"),
        "Expected credentials in output, got: {}",
        output
    );
}

#[test]
fn server_listen_help_should_show_options() {
    let output = Command::new(bin_path())
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
