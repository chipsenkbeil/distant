//! Integration tests for the `distant manager listen` CLI subcommand.
//!
//! Tests starting the manager on a custom socket/pipe and handling duplicates.

use std::process::{Command, Stdio};
use std::time::Duration;

use distant_test_harness::manager::bin_path;

#[cfg(unix)]
#[test]
fn should_listen_on_custom_unix_socket() {
    use assert_fs::prelude::*;

    let temp = assert_fs::TempDir::new().unwrap();
    let socket_path = temp.child("test.sock");

    let mut child = Command::new(bin_path())
        .args(["manager", "listen", "--unix-socket"])
        .arg(socket_path.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn manager");

    // Give the manager time to start and create the socket
    // (socket binding is async, 200ms may not be enough — retry up to 2s)
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if socket_path.path().exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let exists = socket_path.path().exists();

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        exists,
        "Expected socket file to be created at {:?}",
        socket_path.path()
    );
}

#[cfg(windows)]
#[test]
fn should_listen_on_custom_windows_pipe() {
    let pipe_name = format!("distant_test_listen_{}", std::process::id());

    let mut child = Command::new(bin_path())
        .args(["manager", "listen", "--windows-pipe", &pipe_name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn manager");

    // Give the manager time to start and bind the named pipe
    std::thread::sleep(Duration::from_millis(500));

    // Verify the manager process is still running (didn't crash on startup)
    let still_running = child
        .try_wait()
        .expect("Failed to check manager status")
        .is_none();

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        still_running,
        "Manager should still be running after starting on custom pipe"
    );
}

#[cfg(unix)]
#[test]
fn should_fail_on_duplicate_socket() {
    use assert_fs::prelude::*;

    let temp = assert_fs::TempDir::new().unwrap();
    let socket_path = temp.child("test.sock");

    // Start first manager
    let mut child1 = Command::new(bin_path())
        .args(["manager", "listen", "--unix-socket"])
        .arg(socket_path.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn first manager");

    // Give first manager time to bind the socket
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if socket_path.path().exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Start second manager on same socket — should fail
    let output = Command::new(bin_path())
        .args(["manager", "listen", "--unix-socket"])
        .arg(socket_path.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run second manager");

    let _ = child1.kill();
    let _ = child1.wait();

    assert!(
        !output.status.success(),
        "Second manager on same socket should fail, but succeeded"
    );
}

#[cfg(windows)]
#[test]
fn should_fail_on_duplicate_pipe() {
    let pipe_name = format!("distant_test_dup_{}", std::process::id());

    // Start first manager
    let mut child1 = Command::new(bin_path())
        .args(["manager", "listen", "--windows-pipe", &pipe_name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn first manager");

    // Give first manager time to bind the pipe
    std::thread::sleep(Duration::from_millis(500));

    // Start second manager on same pipe — should fail
    let output = Command::new(bin_path())
        .args(["manager", "listen", "--windows-pipe", &pipe_name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run second manager");

    let _ = child1.kill();
    let _ = child1.wait();

    assert!(
        !output.status.success(),
        "Second manager on same pipe should fail, but succeeded"
    );
}
