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
fn should_report_all_capabilities_via_json(#[case] backend: Backend) {
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

    let caps = parsed["capabilities"]
        .as_array()
        .expect("Expected 'capabilities' array in JSON output");

    let cap_strings: Vec<&str> = caps
        .iter()
        .map(|v| v.as_str().expect("capability should be a string"))
        .collect();

    // All backends unconditionally report these core capabilities.
    let common = ["exec", "fs_io", "sys_info"];
    for cap in &common {
        assert!(
            cap_strings.contains(cap),
            "Expected capability '{cap}' in server capabilities, got: {cap_strings:?}"
        );
    }

    // Backend-specific capabilities that are always present.
    let backend_specific: &[&str] = match backend {
        Backend::Host => &[
            "fs_perm",
            "fs_search",
            "fs_watch",
            "tcp_tunnel",
            "tcp_rev_tunnel",
        ],
        Backend::Ssh => &["tcp_tunnel", "tcp_rev_tunnel"],
        Backend::Docker => &["fs_perm"],
    };
    for cap in backend_specific {
        assert!(
            cap_strings.contains(cap),
            "Expected capability '{cap}' for {backend:?}, got: {cap_strings:?}"
        );
    }

    let min_expected = common.len() + backend_specific.len();
    assert!(
        cap_strings.len() >= min_expected,
        "Server should report at least {min_expected} capabilities, got {}",
        cap_strings.len()
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
