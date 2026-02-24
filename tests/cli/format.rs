//! Integration tests for output format handling across CLI commands.
//!
//! Tests shell vs JSON format output for status, kill, and error scenarios.

use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn status_json_format_produces_valid_json(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(["status", "--format", "json"])
        .output()
        .expect("Failed to run status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("status --format json should produce valid JSON");
    assert!(
        parsed.is_object(),
        "Expected JSON object from status, got: {parsed}"
    );
}

#[rstest]
#[test_log::test]
fn kill_should_succeed_with_valid_id(ctx: ManagerCtx) {
    // Get the connection ID
    let output = ctx
        .new_std_cmd(["status", "--format", "json"])
        .output()
        .expect("Failed to run status");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let id = parsed
        .as_object()
        .unwrap()
        .keys()
        .next()
        .expect("Should have at least one connection")
        .clone();

    // Kill with the connection ID
    let kill_output = ctx
        .new_std_cmd(["kill"])
        .arg(&id)
        .output()
        .expect("Failed to run kill");

    assert!(
        kill_output.status.success(),
        "kill should succeed, stderr: {}",
        String::from_utf8_lossy(&kill_output.stderr)
    );

    // Verify connection was removed
    let status_output = ctx
        .new_std_cmd(["status", "--format", "json"])
        .output()
        .expect("Failed to run status after kill");

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    let status_parsed: serde_json::Value = serde_json::from_str(status_stdout.trim()).unwrap();
    assert!(
        status_parsed.as_object().unwrap().is_empty(),
        "Expected no connections after kill"
    );
}

#[rstest]
#[test_log::test]
fn kill_without_id_produces_error(ctx: ManagerCtx) {
    // Kill without an ID should produce a clap error (missing required arg)
    let output = ctx
        .new_std_cmd(["kill"])
        .output()
        .expect("Failed to run kill");

    assert!(!output.status.success(), "kill without ID should fail");
}

#[rstest]
#[test_log::test]
fn kill_with_invalid_id_produces_error(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(["kill"])
        .arg("99999")
        .output()
        .expect("Failed to run kill");

    assert!(!output.status.success(), "kill with invalid ID should fail");
}
