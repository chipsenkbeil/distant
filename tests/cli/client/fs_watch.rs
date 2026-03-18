//! Integration tests for the `distant fs watch` CLI subcommand.
//!
//! Tests watching files and directories for changes. Watch is only supported
//! on the Host backend (SSH and Docker return Unsupported).

use std::process::Stdio;
use std::time::{Duration, Instant};

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;
use distant_test_harness::utils::reader::ThreadedReader;

fn wait_for_watching_ready(stderr: &mut ThreadedReader, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Some(line) = stderr.try_read_line_timeout(remaining.min(Duration::from_millis(500)))
            && line.contains("Watching")
        {
            return;
        }
    }
    panic!("Timed out waiting for 'Watching' ready indicator on stderr");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_support_watching_a_single_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(file.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    file.write_str("some text").unwrap();

    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            break;
        }
    }

    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_support_watching_a_directory_recursively(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let file = dir.child("file");
    file.touch().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .args(["--recursive", temp.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    file.write_str("some text").unwrap();

    let path = file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

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

    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    assert!(
        stdout_data.contains(&path),
        "\"{}\" missing {}",
        stdout_data,
        path
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let invalid_path = temp.to_path_buf().join("missing");

    let child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(invalid_path.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    std::thread::sleep(Duration::from_millis(250));

    let output = child
        .wait_with_output()
        .expect("Failed to wait for child to complete");

    assert!(!output.status.success(), "Child unexpectedly succeeded");
    assert!(output.stdout.is_empty(), "Unexpectedly got stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Failed to watch"),
        "Expected 'Failed to watch' in stderr, got: {stderr}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_support_only_filter(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("watched");
    dir.create_dir_all().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .args(["--recursive", "--only", "create"])
        .arg(dir.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    dir.child("newfile.txt").write_str("hello").unwrap();

    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            break;
        }
    }

    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    assert!(
        !stdout_data.is_empty(),
        "Expected create event to be reported with --only create"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_support_except_filter(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .args(["--recursive", "--except", "access"])
        .arg(dir.to_str().unwrap())
        .spawn()
        .expect("Failed to execute");

    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    dir.child("newfile.txt").write_str("hello").unwrap();

    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            break;
        }
    }

    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    assert!(
        !stdout_data.is_empty(),
        "Expected non-access events to still be reported with --except access"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_report_file_creation_in_watched_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("watched");
    dir.create_dir_all().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .args(["--recursive", dir.to_str().unwrap()])
        .spawn()
        .expect("Failed to execute");

    let mut stderr = ThreadedReader::new(child.stderr.take().unwrap());
    let mut stdout = ThreadedReader::new(child.stdout.take().unwrap());
    wait_for_watching_ready(&mut stderr, Duration::from_secs(5));

    let new_file = dir.child("created.txt");
    new_file.write_str("new content").unwrap();

    let new_file_path = new_file
        .to_path_buf()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let mut stdout_data = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(line) = stdout.try_read_line_timeout(Duration::from_millis(500)) {
            stdout_data.push_str(&line);
            if stdout_data.contains(&new_file_path) {
                break;
            }
        }
    }

    child.kill().expect("Failed to terminate process");
    let _ = child.wait();

    assert!(
        stdout_data.contains(&new_file_path),
        "Expected creation event for {new_file_path}, got: {stdout_data}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_watch_for_create_events_via_parity(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(temp.to_str().unwrap())
        .arg("--recursive")
        .arg("--only")
        .arg("create")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn fs watch");

    std::thread::sleep(Duration::from_secs(1));

    temp.child("watched-file.txt")
        .write_str("watch content")
        .unwrap();

    std::thread::sleep(Duration::from_secs(2));

    child.kill().expect("Failed to kill watch process");
    let output = child
        .wait_with_output()
        .expect("Failed to wait for watch process");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("watched-file.txt"),
        "Expected 'watched-file.txt' in watch output, got: {stdout}"
    );
}
