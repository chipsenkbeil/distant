//! Integration tests for the `distant system-info` CLI subcommand.
//!
//! Tests retrieving and displaying system information from the server.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

/// Extracts the value from a `Key: "value"` or `Key: 'c'` line in
/// system-info output.
///
/// Strips the surrounding quotes (double for strings, single for chars)
/// and returns the inner content.
fn parse_field<'a>(stdout: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}: ");
    let raw = stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("Expected '{key}:' in output, got: {stdout}"));
    raw.trim_matches('"').trim_matches('\'')
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_system_info(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["system-info"])
        .output()
        .expect("Failed to run system-info");

    assert!(
        output.status.success(),
        "system-info should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    let family = parse_field(&stdout, "Family");
    let os = parse_field(&stdout, "Operating System");
    let arch = parse_field(&stdout, "Arch");
    let cwd = parse_field(&stdout, "Cwd");
    let path_sep = parse_field(&stdout, "Path Sep");
    let username = parse_field(&stdout, "Username");

    assert!(!cwd.is_empty(), "Cwd should be non-empty, got: {stdout}");
    assert!(
        !username.is_empty(),
        "Username should be non-empty, got: {stdout}"
    );

    match backend {
        Backend::Docker => {
            assert_eq!(family, "unix", "Docker container should report unix family");
            assert_eq!(os, "linux", "Docker container should report linux OS");
            assert_eq!(path_sep, "/", "Docker container should use / separator");
            assert!(
                !arch.is_empty(),
                "Docker container should report a non-empty arch, got: {stdout}"
            );
        }
        Backend::Ssh => {
            // The SSH backend reports family but cannot detect os/arch
            // on the remote side (they are empty strings for Unix targets).
            assert_eq!(
                family,
                std::env::consts::FAMILY,
                "Family should match host OS family"
            );
            assert_eq!(
                path_sep,
                std::path::MAIN_SEPARATOR.to_string(),
                "Path separator should match host separator"
            );
        }
        Backend::Host => {
            assert_eq!(
                family,
                std::env::consts::FAMILY,
                "Family should match host OS family"
            );
            assert_eq!(os, std::env::consts::OS, "OS should match host OS");
            assert_eq!(
                arch,
                std::env::consts::ARCH,
                "Arch should match host architecture"
            );
            let expected_sep = std::path::MAIN_SEPARATOR.to_string();
            assert_eq!(
                path_sep, expected_sep,
                "Path separator should match host separator"
            );
        }
    }
}
