//! Integration tests for the `distant kill` CLI subcommand.
//!
//! Tests terminating an active connection by its ID and error handling
//! for invalid connection IDs.

use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_kill_connection_by_id(ctx: ManagerCtx) {
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
fn should_kill_in_json_format(ctx: ManagerCtx) {
    // Get the connection ID from JSON status
    let status_output = ctx
        .new_std_cmd(vec!["status", "--format", "json"])
        .output()
        .expect("Failed to run status");
    assert!(status_output.status.success());

    let stdout = String::from_utf8_lossy(&status_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let id = parsed
        .as_object()
        .unwrap()
        .keys()
        .next()
        .expect("Should have at least one connection")
        .clone();

    // Kill with --format json
    let child = ctx
        .new_std_cmd(vec!["kill", "--format", "json"])
        .arg(&id)
        .spawn()
        .expect("Failed to spawn kill --format json");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut proc = ApiProcess::new(child, TIMEOUT);

        // Auth handshake: manager auth
        handle_cli_auth(&mut proc).await;

        // Read final JSON output
        let json: serde_json::Value = proc
            .read_json_from_stdout()
            .await
            .expect("Failed to read kill output")
            .expect("Missing kill output");

        assert_eq!(json["type"], "ok");
        assert_eq!(
            json["id"].as_u64().map(|v| v.to_string()),
            Some(id.clone()),
            "Expected kill output id to match, got: {json}"
        );
    });
}

#[rstest]
#[test_log::test]
fn should_fail_with_invalid_id(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(vec!["kill"])
        .arg("99999")
        .output()
        .expect("Failed to run kill");
    assert!(!output.status.success(), "kill with invalid ID should fail");
}
