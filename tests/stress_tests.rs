//! Stress tests for the distant manager/server.
//!
//! Exercises high-volume request handling, multiple clients, abrupt disconnects,
//! and killed interactive shells.

use assert_fs::prelude::*;
use distant_test_harness::manager::*;
use distant_test_harness::scripts::*;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_handle_large_volume_of_requests(ctx: ManagerCtx) {
    // Create a temporary directory to house a file we create and edit
    // with a large volume of requests
    let root = assert_fs::TempDir::new().unwrap();

    // Establish a path to a file we will edit repeatedly
    let path = root.child("file").to_path_buf();

    // Perform many requests of writing a file and reading a file
    for i in 1..100 {
        let _ = ctx
            .new_assert_cmd(["fs", "write"])
            .arg(path.to_str().unwrap())
            .write_stdin(format!("idx: {i}"))
            .assert();

        ctx.new_assert_cmd(["fs", "read"])
            .arg(path.to_str().unwrap())
            .assert()
            .stdout(format!("idx: {i}"));
    }
}

#[rstest]
#[test_log::test]
fn should_handle_wide_spread_of_clients(ctx: ManagerCtx) {
    use std::thread;

    let root = assert_fs::TempDir::new().unwrap();
    let num_clients = 10;

    // Perform N sequential client operations, each writing and reading a unique file
    for i in 0..num_clients {
        let path = root.child(format!("file_{i}")).to_path_buf();
        let content = format!("client_{i}_data");

        // Write file
        let write_output = ctx
            .new_std_cmd(["fs", "write"])
            .arg(path.to_str().unwrap())
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(content.as_bytes()).ok();
                }
                drop(child.stdin.take());
                child.wait_with_output()
            });

        assert!(
            write_output.is_ok() && write_output.as_ref().unwrap().status.success(),
            "Client {i} write failed"
        );

        // Small delay to let filesystem settle
        thread::sleep(std::time::Duration::from_millis(50));

        // Read back and verify
        let read_output = ctx
            .new_std_cmd(["fs", "read"])
            .arg(path.to_str().unwrap())
            .output()
            .expect("Failed to run read");

        assert!(
            read_output.status.success(),
            "Client {i} read failed: {}",
            String::from_utf8_lossy(&read_output.stderr)
        );

        let stdout = String::from_utf8_lossy(&read_output.stdout);
        assert_eq!(stdout.trim(), content, "Client {i} read mismatch");
    }
}

#[rstest]
#[test_log::test]
fn should_handle_abrupt_client_disconnects(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    let path = root.child("file").to_path_buf();

    // Write a file to establish that the server is healthy
    let _ = ctx
        .new_assert_cmd(["fs", "write"])
        .arg(path.to_str().unwrap())
        .write_stdin("before disconnect")
        .assert()
        .success();

    // Spawn a long-running operation and kill it mid-flight
    let mut child = ctx
        .new_std_cmd(["spawn"])
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(ECHO_STDIN_TO_STDOUT.to_str().unwrap())
        .spawn()
        .expect("Failed to spawn process");

    // Kill the child abruptly without waiting for completion
    let _ = child.kill();
    let _ = child.wait();

    // Give the server a moment to recover
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify the manager/server is still healthy by performing another operation
    let _ = ctx
        .new_assert_cmd(["fs", "write"])
        .arg(path.to_str().unwrap())
        .write_stdin("after disconnect")
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "read"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("after disconnect");
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_handle_badly_killing_client_shell_with_interactive_process(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    let path = root.child("file").to_path_buf();

    // Start a shell with an interactive process (cat waits indefinitely for input)
    let mut child = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "cat"])
        .spawn()
        .expect("Failed to spawn shell with interactive process");

    // Give the process time to start
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Kill the client process abruptly (simulates user hitting Ctrl+C or terminal closing)
    let _ = child.kill();
    let _ = child.wait();

    // Give the server time to clean up the orphaned process
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify the server is still healthy
    let _ = ctx
        .new_assert_cmd(["fs", "write"])
        .arg(path.to_str().unwrap())
        .write_stdin("still alive")
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "read"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("still alive");
}

#[cfg(windows)]
#[rstest]
#[test_log::test]
fn should_handle_badly_killing_client_shell_with_interactive_process(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    let path = root.child("file").to_path_buf();

    // Start a shell with an interactive process (findstr /v "" waits indefinitely for input)
    let mut child = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "findstr", "/v", "\"\""])
        .spawn()
        .expect("Failed to spawn shell with interactive process");

    // Give the process time to start
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Kill the client process abruptly (simulates user closing terminal)
    let _ = child.kill();
    let _ = child.wait();

    // Give the server time to clean up the orphaned process
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify the server is still healthy
    let _ = ctx
        .new_assert_cmd(["fs", "write"])
        .arg(path.to_str().unwrap())
        .write_stdin("still alive")
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "read"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("still alive");
}
