//! Integration tests for the `distant select` CLI subcommand.
//!
//! Tests selecting/switching the active connection by its ID.
//! Host-only since select operates on the manager's connection list.

use rstest::*;
use serde_json::json;

use distant_test_harness::backend::Backend;
use distant_test_harness::manager::{ApiProcess, TIMEOUT, handle_cli_auth};
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_select_connection_by_id(#[case] backend: Backend) {
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

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_select_in_json_format(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let child = ctx
        .new_std_cmd(vec!["select", "--format", "json"])
        .spawn()
        .expect("Failed to spawn select --format json");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut proc = ApiProcess::new(child, TIMEOUT);

        handle_cli_auth(&mut proc).await;

        let select_msg: serde_json::Value = proc
            .read_json_from_stdout()
            .await
            .expect("Failed to read select prompt")
            .expect("Missing select prompt");

        assert_eq!(select_msg["type"], "select");
        let choices = select_msg["choices"]
            .as_array()
            .expect("Expected choices array");
        assert!(!choices.is_empty(), "Expected at least one choice");
        assert!(
            select_msg["current"].is_number(),
            "Expected numeric current, got: {select_msg}"
        );

        proc.write_json_to_stdin(json!({"type": "selected", "choice": 0}))
            .await
            .expect("Failed to write selection response");
    });
}
