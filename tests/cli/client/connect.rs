//! Integration tests for the `distant connect` CLI subcommand.
//!
//! Tests connecting to a server in various formats, reusing connections,
//! forcing new connections, and error handling.

use rstest::*;

use distant_test_harness::manager::*;

/// The server listens on 0.0.0.0, but on Windows connecting to 0.0.0.0 fails
/// with "The requested address is not valid in its context" (os error 10049).
/// Replace 0.0.0.0 with 127.0.0.1 in the credentials for portability.
fn fix_credentials(creds: &str) -> String {
    creds.replace("0.0.0.0", "127.0.0.1")
}

#[rstest]
#[test_log::test]
fn should_connect_in_default_format(manager_only_ctx: ManagerOnlyCtx) {
    let creds = fix_credentials(&manager_only_ctx.credentials);
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg(&creds)
        .output()
        .expect("Failed to run connect");

    assert!(
        output.status.success(),
        "connect should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Shell format outputs the connection ID
    assert!(
        !stdout.trim().is_empty(),
        "Expected non-empty connect output, got empty"
    );
}

#[rstest]
#[test_log::test]
fn should_connect_and_show_in_status(manager_only_ctx: ManagerOnlyCtx) {
    let creds = fix_credentials(&manager_only_ctx.credentials);

    // Connect to the server
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg(&creds)
        .output()
        .expect("Failed to run connect");

    assert!(
        output.status.success(),
        "connect should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the connection shows up in status
    let status_output = manager_only_ctx
        .new_std_cmd(["status", "--format", "json"])
        .output()
        .expect("Failed to run status");

    assert!(status_output.status.success());
    let stdout = String::from_utf8_lossy(&status_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        !parsed.as_object().unwrap().is_empty(),
        "Expected at least one connection after connect"
    );
}

#[rstest]
#[test_log::test]
fn should_connect_in_json_format(manager_only_ctx: ManagerOnlyCtx) {
    let creds = fix_credentials(&manager_only_ctx.credentials);
    let child = manager_only_ctx
        .new_std_cmd(["connect", "--format", "json"])
        .arg(&creds)
        .spawn()
        .expect("Failed to spawn connect --format json");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut proc = ApiProcess::new(child, TIMEOUT);

        // Manager auth handshake (none)
        handle_cli_auth(&mut proc).await;

        // Read final JSON output
        let json = proc
            .read_json_from_stdout()
            .await
            .expect("Failed to read connect output")
            .expect("Missing connect output");

        assert_eq!(json["type"], "connected");
        assert!(json["id"].is_number(), "Expected numeric id, got: {json}");
        assert!(
            json["reused"].is_boolean(),
            "Expected boolean reused, got: {json}"
        );
    });
}

#[rstest]
#[test_log::test]
fn should_fail_with_invalid_destination(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg("distant://nonexistent:99999")
        .output()
        .expect("Failed to run connect");

    assert!(
        !output.status.success(),
        "connect to invalid destination should fail"
    );
}
