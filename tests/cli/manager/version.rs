//! Integration tests for the `distant manager version` CLI subcommand.
//!
//! Verifies the version output matches the compile-time `CARGO_PKG_VERSION`.

use std::process::Stdio;

use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_output_version(ctx: ManagerCtx) {
    ctx.new_assert_cmd(vec!["manager", "version"])
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")));
}

#[rstest]
#[test_log::test]
fn should_output_version_in_json_format(ctx: ManagerCtx) {
    let output = ctx
        .new_std_cmd(["manager", "version", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .expect("Failed to run manager version --format json");

    assert!(
        output.status.success(),
        "manager version --format json should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("manager version --format json should produce valid JSON");

    // Manager version JSON format is { "version": "<version>" }
    assert!(parsed.is_object(), "Expected JSON object, got: {parsed}");
    let obj = parsed.as_object().unwrap();
    assert!(
        obj.contains_key("version"),
        "Expected 'version' field in JSON, got keys: {:?}",
        obj.keys().collect::<Vec<_>>(),
    );
    assert_eq!(
        obj["version"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION"),
        "Version doesn't match CARGO_PKG_VERSION",
    );
}
