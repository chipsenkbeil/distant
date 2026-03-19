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
        stdout.contains("Type:") || stdout.contains("type"),
        "Expected metadata output containing type info, got: {stdout}"
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
        stdout.contains("Type:") || stdout.contains("type"),
        "Expected metadata output containing type info, got: {stdout}"
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

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_including_a_canonicalized_path(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    use distant_test_harness::utils::regex_pred;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    ctx.new_assert_cmd(["fs", "metadata"])
        .args(["--canonicalize", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(&format!(
            concat!(
                "Canonicalized Path: {}\n",
                "Type: symlink\n",
                "Len: .*\n",
                "Readonly: false\n",
                "Created: .*\n",
                "Last Accessed: .*\n",
                "Last Modified: .*\n",
            ),
            regex::escape(&format!("{:?}", file.path().canonicalize().unwrap()))
        )));
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_resolving_file_type_of_symlink(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    use distant_test_harness::utils::regex_pred;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    ctx.new_assert_cmd(["fs", "metadata"])
        .args(["--resolve-file-type", link.to_str().unwrap()])
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
