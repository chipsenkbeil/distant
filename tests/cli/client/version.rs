//! Integration tests for the `distant version` CLI subcommand.
//!
//! Tests displaying version information.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_version(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version");

    assert!(
        output.status.success(),
        "version should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_capabilities(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Client:"),
        "Expected 'Client:' in version output, got: {stdout}"
    );
    assert!(
        stdout.contains("Server:"),
        "Expected 'Server:' in version output, got: {stdout}"
    );
    assert!(
        stdout.contains("Capabilities"),
        "Expected 'Capabilities' in version output, got: {stdout}"
    );
    assert!(
        stdout.contains("+exec"),
        "Expected '+exec' capability in version output, got: {stdout}"
    );
    assert!(
        stdout.contains("+fs_io"),
        "Expected '+fs_io' capability in version output, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_json_format_flag(#[case] backend: Backend) {
    use std::process::Stdio;

    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["version", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .expect("Failed to run version --format json");

    assert!(
        output.status.success(),
        "version --format json should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("version --format json should produce valid JSON");

    assert!(parsed.is_object(), "Expected JSON object, got: {parsed}");
    let obj = parsed.as_object().unwrap();
    assert!(
        obj.contains_key("server_version"),
        "Expected 'server_version' field in JSON, got keys: {:?}",
        obj.keys().collect::<Vec<_>>(),
    );
    assert!(
        obj.contains_key("protocol_version"),
        "Expected 'protocol_version' field in JSON, got keys: {:?}",
        obj.keys().collect::<Vec<_>>(),
    );
}
