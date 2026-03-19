//! Integration tests for the `distant fs exists` CLI subcommand.
//!
//! Tests checking whether a path exists on the filesystem.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_true_if_file_exists(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("exists");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "exists-test.txt");
    ctx.cli_write(&path, "exists");

    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(&path)
        .output()
        .expect("Failed to run fs exists");

    assert!(
        output.status.success(),
        "fs exists should succeed for existing file, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("true"),
        "Expected 'true' for existing file, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_false_if_not_exists(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("exists-false");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "nonexistent");

    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(&path)
        .output()
        .expect("Failed to run fs exists");

    assert!(output.status.success(), "fs exists should always exit zero");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("false"),
        "Expected 'false' for nonexistent file, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_true_for_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("exists-dir");
    ctx.cli_mkdir(&dir);

    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs exists");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("true"),
        "Expected 'true' for existing directory, got: {stdout}"
    );
}
