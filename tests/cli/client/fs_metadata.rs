//! Integration tests for the `distant fs metadata` CLI subcommand.
//!
//! Tests retrieving metadata for files and directories.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_metadata_for_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("metadata");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "metadata-test.txt");
    ctx.cli_write(&path, "metadata content");

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&path)
        .output()
        .expect("Failed to run fs metadata");

    assert!(
        output.status.success(),
        "fs metadata should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Type: file"),
        "Expected 'Type: file' in metadata output, got: {stdout}"
    );
    assert!(
        stdout.contains("Len:"),
        "Expected 'Len:' in metadata output, got: {stdout}"
    );
    assert!(
        stdout.contains("Readonly:"),
        "Expected 'Readonly:' in metadata output, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_output_metadata_for_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("metadata-dir");
    ctx.cli_mkdir(&dir);
    let sub = ctx.child_path(&dir, "sub");
    ctx.cli_mkdir(&sub);

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&sub)
        .output()
        .expect("Failed to run fs metadata");

    assert!(
        output.status.success(),
        "fs metadata should succeed for directory, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Type: dir"),
        "Expected 'Type: dir' in metadata output, got: {stdout}"
    );
    assert!(
        stdout.contains("Readonly:"),
        "Expected 'Readonly:' in metadata output, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("metadata-err");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "nonexistent");

    ctx.new_assert_cmd(["fs", "metadata"])
        .arg(&path)
        .assert()
        .code(1);
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_including_a_canonicalized_path(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("metadata-canon");
    ctx.cli_mkdir(&dir);
    let file_path = ctx.child_path(&dir, "file");
    ctx.cli_write(&file_path, "");
    let link_path = ctx.child_path(&dir, "link");
    ctx.cli_symlink(&file_path, &link_path);

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .args(["--canonicalize", &link_path])
        .output()
        .expect("Failed to run fs metadata");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Canonicalized Path:"),
        "Expected 'Canonicalized Path:' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Type: symlink"),
        "Expected 'Type: symlink' in output, got: {stdout}"
    );
    assert!(
        stdout.contains("Readonly:"),
        "Expected 'Readonly:' in output, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_resolving_file_type_of_symlink(#[case] backend: Backend) {
    use distant_test_harness::utils::regex_pred;

    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("metadata-resolve");
    ctx.cli_mkdir(&dir);
    let file_path = ctx.child_path(&dir, "file");
    ctx.cli_write(&file_path, "");
    let link_path = ctx.child_path(&dir, "link");
    ctx.cli_symlink(&file_path, &link_path);

    ctx.new_assert_cmd(["fs", "metadata"])
        .args(["--resolve-file-type", &link_path])
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: file\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )));
}
