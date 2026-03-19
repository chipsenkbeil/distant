//! Integration tests for the `distant system-info` CLI subcommand.
//!
//! Tests retrieving and displaying system information from the server.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

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
    assert!(
        stdout.contains("Family:"),
        "Expected 'Family:' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Operating System:"),
        "Expected 'Operating System:' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Arch:"),
        "Expected 'Arch:' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Cwd:"),
        "Expected 'Cwd:' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Path Sep:"),
        "Expected 'Path Sep:' in output, got: {stdout}"
    );

    match backend {
        Backend::Docker => {
            assert!(
                stdout.contains("\"unix\""),
                "Docker container should report unix family, got: {stdout}"
            );
        }
        _ => {
            if cfg!(unix) {
                assert!(
                    stdout.contains("\"unix\""),
                    "Unix host should report unix family, got: {stdout}"
                );
            } else {
                assert!(
                    stdout.contains("\"windows\""),
                    "Windows host should report windows family, got: {stdout}"
                );
            }
        }
    }
}
