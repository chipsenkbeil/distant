//! E2E tests for global CLI options: `--log-level` and `--log-file`.
//!
//! `--config` is tested in `config.rs`.

use std::process::Command;

use assert_fs::prelude::PathChild as _;

use distant_test_harness::manager::{self, HostManagerCtx};

/// Build a `distant version` command with custom log settings, bypassing the
/// default `--log-file` / `--log-level` injected by `new_std_cmd`.
fn build_version_cmd_with_log(ctx: &HostManagerCtx, log_level: &str, log_file: &str) -> Command {
    let mut cmd = Command::new(manager::bin_path());
    cmd.arg("version")
        .arg("--log-file")
        .arg(log_file)
        .arg("--log-level")
        .arg(log_level);

    if cfg!(windows) {
        cmd.arg("--windows-pipe").arg(ctx.socket_or_pipe());
    } else {
        cmd.arg("--unix-socket").arg(ctx.socket_or_pipe());
    }

    cmd
}

#[tokio::test]
async fn log_level_trace_produces_verbose_log() {
    let ctx = HostManagerCtx::start();
    let temp = assert_fs::TempDir::new().unwrap();
    let log_file = temp.child("trace.log");

    let output = build_version_cmd_with_log(&ctx, "trace", log_file.to_str().unwrap())
        .output()
        .expect("Failed to run version with trace logging");

    assert!(
        output.status.success(),
        "version should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let log_contents =
        std::fs::read_to_string(log_file.path()).expect("Failed to read trace log file");
    assert!(
        !log_contents.is_empty(),
        "Trace log file should contain output"
    );
    assert!(
        log_contents.contains("TRACE"),
        "Trace log should contain at least one TRACE-level entry"
    );
}

#[tokio::test]
async fn log_level_error_produces_minimal_log() {
    let ctx = HostManagerCtx::start();
    let temp = assert_fs::TempDir::new().unwrap();
    let trace_log = temp.child("trace.log");
    let error_log = temp.child("error.log");

    // Run with trace level
    build_version_cmd_with_log(&ctx, "trace", trace_log.to_str().unwrap())
        .output()
        .expect("Failed to run version with trace logging");

    // Run with error level
    build_version_cmd_with_log(&ctx, "error", error_log.to_str().unwrap())
        .output()
        .expect("Failed to run version with error logging");

    let trace_len = std::fs::metadata(trace_log.path())
        .map(|m| m.len())
        .unwrap_or(0);
    let error_len = std::fs::metadata(error_log.path())
        .map(|m| m.len())
        .unwrap_or(0);

    assert!(
        trace_len > error_len,
        "Trace log ({trace_len} bytes) should be larger than error log ({error_len} bytes)"
    );

    let error_contents = std::fs::read_to_string(error_log.path()).unwrap_or_default();
    for excluded_level in ["INFO", "WARN", "DEBUG", "TRACE"] {
        assert!(
            !error_contents.contains(excluded_level),
            "Error-level log should not contain {excluded_level} entries"
        );
    }
}

#[tokio::test]
async fn log_file_is_created_at_specified_path() {
    let ctx = HostManagerCtx::start();
    let temp = assert_fs::TempDir::new().unwrap();
    let log_file = temp.child("custom.log");

    assert!(
        !log_file.path().exists(),
        "Log file should not exist before running command"
    );

    let output = build_version_cmd_with_log(&ctx, "info", log_file.to_str().unwrap())
        .output()
        .expect("Failed to run version with custom log file");

    assert!(
        output.status.success(),
        "version should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        log_file.path().exists(),
        "Log file should be created at the specified path"
    );
}
