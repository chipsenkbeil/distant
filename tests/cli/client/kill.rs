//! Integration tests for the `distant kill` CLI subcommand.
//!
//! Tests terminating an active connection by its ID and error handling
//! for invalid connection IDs. Kill operates on the manager's connection
//! list, so these tests use Host backend only.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_kill_connection_by_id(#[case] backend: Backend) {
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
#[case::host(Backend::Host)]
#[test_log::test]
fn should_fail_with_invalid_id(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(vec!["kill"])
        .arg("99999")
        .output()
        .expect("Failed to run kill");
    assert!(!output.status.success(), "kill with invalid ID should fail");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_kill_and_verify_commands_fail(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version");
    assert!(
        output.status.success(),
        "version should succeed before kill"
    );

    let status_output = ctx
        .new_std_cmd(["status"])
        .output()
        .expect("Failed to run status");

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    let status_stderr = String::from_utf8_lossy(&status_output.stderr);

    let combined = format!("{status_stdout}\n{status_stderr}");
    let conn_id = combined
        .lines()
        .find_map(|line| {
            let trimmed = line.trim().strip_prefix("* ").unwrap_or(line.trim());
            if !trimmed.contains(" -> ") {
                return None;
            }
            trimmed.split_whitespace().next()
        })
        .unwrap_or_else(|| {
            panic!(
                "Failed to find connection ID in status output.\nstdout: {status_stdout}\nstderr: {status_stderr}"
            )
        });

    ctx.new_assert_cmd(["kill"]).arg(conn_id).assert().success();

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version after kill");

    assert!(
        !output.status.success(),
        "version should fail after connection is killed"
    );
}
