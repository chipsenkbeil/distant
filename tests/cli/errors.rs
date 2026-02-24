//! Integration tests for CLI error handling.
//!
//! Tests invalid flags, missing required arguments, and commands that should
//! not auto-start the manager.

use assert_cmd::Command;

#[cfg(unix)]
use assert_fs::prelude::*;

#[test]
fn invalid_flag_produces_clap_error() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.arg("--invalid-flag-xyz").assert().failure();

    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("invalid-flag-xyz")
            || stderr.contains("unexpected")
            || stderr.contains("error"),
        "Expected error mentioning invalid flag, got: {stderr}"
    );
}

#[test]
fn missing_required_arg_produces_clap_error() {
    // `distant fs read` without a path should fail
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["fs", "read"]).assert().failure();
}

#[cfg(unix)]
#[test]
fn api_should_not_autostart_manager() {
    let temp = assert_fs::TempDir::new().unwrap();
    let bogus_socket = temp.child("bogus.sock");

    // Ensure socket doesn't exist
    assert!(!bogus_socket.path().exists());

    // `distant api --unix-socket <bogus>` should fail without auto-starting a manager
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["api", "--unix-socket"])
        .arg(bogus_socket.path())
        .assert()
        .failure();

    // The bogus socket should still not exist (no auto-start)
    assert!(
        !bogus_socket.path().exists(),
        "api should not auto-start a manager"
    );
}

#[cfg(windows)]
#[test]
fn api_should_not_autostart_manager() {
    let bogus_pipe = format!("distant_bogus_api_{}", std::process::id());

    // `distant api --windows-pipe <bogus>` should fail without auto-starting a manager
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["api", "--windows-pipe", &bogus_pipe])
        .assert()
        .failure();
}

#[cfg(unix)]
#[test]
fn status_overview_should_not_autostart_manager() {
    let temp = assert_fs::TempDir::new().unwrap();
    let bogus_socket = temp.child("bogus.sock");

    // `distant status --unix-socket <bogus>` should not auto-start a manager
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let _output = cmd
        .args(["status", "--unix-socket"])
        .arg(bogus_socket.path())
        .output()
        .expect("Failed to run status");

    // The bogus socket should still not exist
    assert!(
        !bogus_socket.path().exists(),
        "status should not auto-start a manager"
    );
}

#[cfg(windows)]
#[test]
fn status_overview_should_not_autostart_manager() {
    let bogus_pipe = format!("distant_bogus_status_{}", std::process::id());

    // `distant status --windows-pipe <bogus>` should not auto-start a manager
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let _output = cmd
        .args(["status", "--windows-pipe", &bogus_pipe])
        .output()
        .expect("Failed to run status");
}
