use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_select_connection_by_id(ctx: ManagerCtx) {
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

    // Select it — need to use new_std_cmd for dynamic args
    let select_output = ctx
        .new_std_cmd(vec!["select"])
        .arg(&id)
        .output()
        .expect("Failed to run select");
    assert!(
        select_output.status.success(),
        "select should succeed, stderr: {}",
        String::from_utf8_lossy(&select_output.stderr)
    );
}
