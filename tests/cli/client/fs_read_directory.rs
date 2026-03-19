//! Integration tests for the `distant fs read` CLI subcommand when used on directories.
//!
//! Tests listing directory contents.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_list_directory_entries(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "aaa.txt"), "a");
    ctx.cli_write(&ctx.child_path(&dir, "bbb.txt"), "b");

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs read (directory)");

    assert!(
        output.status.success(),
        "fs read (directory) should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("aaa.txt"),
        "Expected 'aaa.txt' in directory listing, got: {stdout}"
    );
    assert!(
        stdout.contains("bbb.txt"),
        "Expected 'bbb.txt' in directory listing, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_list_subdirectories(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-sub");
    ctx.cli_mkdir(&dir);

    let sub = ctx.child_path(&dir, "subdir");
    ctx.cli_mkdir(&sub);
    ctx.cli_write(&ctx.child_path(&dir, "file.txt"), "content");

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs read (directory)");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("subdir"),
        "Expected 'subdir' in directory listing, got: {stdout}"
    );
    assert!(
        stdout.contains("file.txt"),
        "Expected 'file.txt' in directory listing, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-err");
    ctx.cli_mkdir(&dir);
    let missing = ctx.child_path(&dir, "missing-dir");

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&missing)
        .assert()
        .code(1);
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_use_absolute_paths_if_specified(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-abs");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "file1"), "");
    ctx.cli_write(&ctx.child_path(&dir, "file2"), "");

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .args(["--absolute", &dir])
        .output()
        .expect("Failed to run fs read --absolute");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&ctx.child_path(&dir, "file1")),
        "Expected absolute path to file1 in output, got: {stdout}"
    );
    assert!(
        stdout.contains(&ctx.child_path(&dir, "file2")),
        "Expected absolute path to file2 in output, got: {stdout}"
    );
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_canonicalize_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-canon");
    ctx.cli_mkdir(&dir);
    let target = ctx.child_path(&dir, "target_dir");
    ctx.cli_mkdir(&target);
    ctx.cli_write(&ctx.child_path(&target, "file1"), "");
    let link = ctx.child_path(&dir, "link");

    let ln_output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "ln", "-s", &target, &link])
        .output()
        .expect("Failed to create symlink");
    assert!(
        ln_output.status.success(),
        "ln -s failed: {}",
        String::from_utf8_lossy(&ln_output.stderr)
    );

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .args(["--canonicalize", "--absolute", &link])
        .output()
        .expect("Failed to run fs read --canonicalize");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("file1"),
        "Expected canonicalized listing to contain 'file1', got: {stdout}"
    );
}
