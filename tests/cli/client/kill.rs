use rstest::*;

use crate::common::fixtures::*;

#[rstest]
#[test_log::test]
fn should_kill_connection_by_id(ctx: DistantManagerCtx) {
    // Get the connection ID from JSON status
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

    // Kill it — need to use new_std_cmd for dynamic args
    let kill_output = ctx
        .new_std_cmd(vec!["kill"])
        .arg(&id)
        .output()
        .expect("Failed to run kill");
    assert!(
        kill_output.status.success(),
        "kill should succeed, stderr: {}",
        String::from_utf8_lossy(&kill_output.stderr)
    );

    // Verify it's gone
    let output = ctx
        .new_assert_cmd(vec!["status", "--format", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        parsed.as_object().unwrap().is_empty(),
        "Expected no connections after kill, got: {parsed}"
    );
}

#[rstest]
#[test_log::test]
fn should_fail_with_invalid_id(ctx: DistantManagerCtx) {
    let output = ctx
        .new_std_cmd(vec!["kill"])
        .arg("99999")
        .output()
        .expect("Failed to run kill");
    assert!(!output.status.success(), "kill with invalid ID should fail");
}
