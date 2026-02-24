//! Integration tests for the `distant launch` CLI subcommand.
//!
//! Tests launching a local server and error handling for invalid binary paths.

use rstest::*;

use distant_test_harness::manager::{ManagerOnlyCtx, bin_path, manager_only_ctx};

#[rstest]
#[test_log::test]
fn should_launch_local_server(manager_only_ctx: ManagerOnlyCtx) {
    // distant launch --distant <bin_path> distant://localhost
    // We must pass --distant so that the launch command can find the binary
    // (it's not on PATH in CI release builds)
    let output = manager_only_ctx
        .new_std_cmd(["launch"])
        .arg("--distant")
        .arg(bin_path())
        .arg("distant://localhost")
        .output()
        .expect("Failed to run launch");

    assert!(
        output.status.success(),
        "launch should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify a connection was created by checking status
    let status_output = manager_only_ctx
        .new_std_cmd(["status", "--format", "json"])
        .output()
        .expect("Failed to run status");

    assert!(status_output.status.success());
    let stdout = String::from_utf8_lossy(&status_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        !parsed.as_object().unwrap().is_empty(),
        "Expected at least one connection after launch"
    );
}

#[rstest]
#[test_log::test]
fn should_fail_when_binary_not_found(manager_only_ctx: ManagerOnlyCtx) {
    // distant launch --distant /nonexistent/path distant://localhost
    let output = manager_only_ctx
        .new_std_cmd(["launch"])
        .args(["--distant", "/nonexistent/distant_binary"])
        .arg("distant://localhost")
        .output()
        .expect("Failed to run launch");

    assert!(
        !output.status.success(),
        "launch with nonexistent binary should fail"
    );
}
