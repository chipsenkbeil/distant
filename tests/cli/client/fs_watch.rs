//! Integration tests for the `distant fs watch` CLI subcommand.
//!
//! Tests watching a single file for changes, watching a directory recursively,
//! and error handling when watching a non-existent path.

use std::time::{Duration, Instant};

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;
use distant_test_harness::utils::reader::ThreadedReader;

fn wait_a_bit() {
    wait_millis(250);
}

fn wait_millis(millis: u64) {
    std::thread::sleep(Duration::from_millis(millis));
}

/// Read stderr lines until one containing "Watching" appears, or panic after timeout.
fn wait_for_watching_ready(stderr: &mut ThreadedReader, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Some(line) = stderr.try_read_line_timeout(remaining.min(Duration::from_millis(500)))
        {
            if line.contains("Watching") {
                return;
            }
        }
    }
    panic!("Timed out waiting for 'Watching' ready indicator on stderr");
}

#[rstest]
#[test_log::test]
fn should_support_watching_a_single_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    // distant fs watch {path}
    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(file.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    // Wait for watcher to be ready by reading stderr until "Watching" appears
    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    // Now manipulate the file (watcher is guaranteed ready)
    file.write_str("some text").unwrap();

    // Read stdout with generous timeout
    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            break;
        }
    }

    // Close out the process
    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Verify we get information printed out about the change
    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
}

#[rstest]
#[test_log::test]
fn should_support_watching_a_directory_recursively(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    // distant fs watch {path}
    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .args(["--recursive", temp.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    // Wait for watcher to be ready by reading stderr until "Watching" appears
    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    // Now manipulate the file (watcher is guaranteed ready)
    file.write_str("some text").unwrap();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Read stdout with generous timeout, collecting lines until we see the file path
    // (on Windows, a recursive watch may report parent directory changes before the file)
    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            if stdout_data.contains(&path) {
                break;
            }
        }
    }

    // Close out the process
    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    // Verify we get information printed out about the change
    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let invalid_path = temp.to_path_buf().join("missing");

    // distant fs watch {path}
    let child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(invalid_path.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    // Pause a bit to ensure that the process started and failed
    wait_a_bit();

    let output = child
        .wait_with_output()
        .expect("Failed to wait for child to complete");

    // Verify we get information printed out about the change
    assert!(!output.status.success(), "Child unexpectedly succeeded");
    assert!(output.stdout.is_empty(), "Unexpectedly got stdout");
    assert!(!output.stderr.is_empty(), "Missing stderr output");
}
