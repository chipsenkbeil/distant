//! Integration tests for the `distant status` CLI subcommand.
//!
//! Tests displaying active connections in JSON format and querying detail
//! for a specific connection by ID. Host-only since status queries the
//! manager's connection list.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_output_connections_in_json_mode(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_assert_cmd(vec!["status", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");

    assert!(parsed.is_object(), "Expected JSON object, got: {parsed}");
    assert!(
        !parsed.as_object().unwrap().is_empty(),
        "Expected at least one connection in JSON output"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_output_detail_for_specific_connection(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_assert_cmd(vec!["status", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let id = parsed
        .as_object()
        .unwrap()
        .keys()
        .next()
        .expect("Should have at least one connection")
        .clone();

    let detail_output = ctx
        .new_std_cmd(vec!["status"])
        .arg(&id)
        .output()
        .expect("Failed to run status <id>");

    assert!(
        detail_output.status.success(),
        "status <id> should succeed, stderr: {}",
        String::from_utf8_lossy(&detail_output.stderr),
    );
}
