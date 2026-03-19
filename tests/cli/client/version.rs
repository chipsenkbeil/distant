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
#[test_log::test]
fn should_output_capabilities(#[case] backend: Backend) {
    use distant_core::protocol::{PROTOCOL_VERSION, semver};

    use distant_test_harness::utils::predicates_ext::TrimmedLinesMatchPredicate;

    let ctx = skip_if_no_backend!(backend);

    // Safety: CARGO_PKG_VERSION is always set during build
    let version: semver::Version = env!("CARGO_PKG_VERSION").parse().unwrap();

    let client_version = if version.build.is_empty() {
        let mut version = version.clone();
        version.build = semver::BuildMetadata::new(env!("CARGO_PKG_NAME")).unwrap();
        version
    } else {
        let mut version = version.clone();
        let raw_build_str = format!("{}.{}", version.build.as_str(), env!("CARGO_PKG_NAME"));
        version.build = semver::BuildMetadata::new(&raw_build_str).unwrap();
        version
    };

    let server_version = if version.build.is_empty() {
        let mut version = version;
        version.build = semver::BuildMetadata::new("distant-host").unwrap();
        version
    } else {
        let raw_build_str = format!("{}.{}", version.build.as_str(), "distant-host");
        let mut version = version;
        version.build = semver::BuildMetadata::new(&raw_build_str).unwrap();
        version
    };

    let expected = indoc::formatdoc! {"
        Client: {client_version} (Protocol {PROTOCOL_VERSION})
        Server: {server_version} (Protocol {PROTOCOL_VERSION})
        Capabilities supported (+) or not (-):
        +exec           +fs_io          +fs_perm        +fs_search
        +fs_watch       +sys_info       +tcp_rev_tunnel +tcp_tunnel
    "};

    ctx.cmd("version")
        .assert()
        .success()
        .stdout(TrimmedLinesMatchPredicate::new(expected));
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
