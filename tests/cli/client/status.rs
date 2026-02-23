use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_output_connections_in_json_mode(ctx: ManagerCtx) {
    let output = ctx
        .new_assert_cmd(vec!["status", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");

    // The JSON output should be a map of connection id -> destination
    assert!(parsed.is_object(), "Expected JSON object, got: {parsed}");
    assert!(
        !parsed.as_object().unwrap().is_empty(),
        "Expected at least one connection in JSON output"
    );
}

#[rstest]
#[test_log::test]
fn should_output_detail_for_specific_connection(ctx: ManagerCtx) {
    // First get the connection ID from JSON status
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

    // Now get detail for that specific connection — need to use new_std_cmd
    // since new_assert_cmd only accepts &'static str
    let detail_output = ctx
        .new_std_cmd(vec!["status"])
        .arg(&id)
        .output()
        .expect("Failed to run status <id>");

    assert!(
        detail_output.status.success(),
        "status <id> should succeed, stderr: {}",
        String::from_utf8_lossy(&detail_output.stderr)
    );
}
