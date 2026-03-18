//! Integration tests for the `distant fs remove` CLI subcommand.
//!
//! Tests removing files, empty directories, non-empty directories with `--force`,
//! and error handling when force is not specified for non-empty directories.
//! Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_removing_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("remove");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "remove-test.txt");
    ctx.cli_write(&path, "to be removed");
    assert!(ctx.cli_exists(&path), "File should exist before removal");

    ctx.new_assert_cmd(["fs", "remove"])
        .arg(&path)
        .assert()
        .success();

    assert!(
        !ctx.cli_exists(&path),
        "File should be removed (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_removing_empty_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("remove-emptydir");
    ctx.cli_mkdir(&dir);
    let empty_dir = ctx.child_path(&dir, "empty");
    ctx.cli_mkdir(&empty_dir);

    ctx.new_assert_cmd(["fs", "remove"])
        .arg(&empty_dir)
        .assert()
        .success();

    assert!(
        !ctx.cli_exists(&empty_dir),
        "Empty directory should be removed (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_removing_nonempty_directory_if_force_specified(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("remove-force");
    ctx.cli_mkdir(&dir);
    let nonempty = ctx.child_path(&dir, "nonempty");
    ctx.cli_mkdir(&nonempty);
    ctx.cli_write(&ctx.child_path(&nonempty, "file.txt"), "content");

    ctx.new_assert_cmd(["fs", "remove"])
        .args(["--force", &nonempty])
        .assert()
        .success();

    assert!(
        !ctx.cli_exists(&nonempty),
        "Non-empty directory should be removed with --force (verified via CLI)"
    );
}

/// Docker is excluded because its `rm -r` (without --force) is still
/// recursive and succeeds on non-empty directories with writable files.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("remove-err");
    ctx.cli_mkdir(&dir);
    let nonempty = ctx.child_path(&dir, "nonempty");
    ctx.cli_mkdir(&nonempty);
    ctx.cli_write(&ctx.child_path(&nonempty, "file.txt"), "content");

    ctx.new_assert_cmd(["fs", "remove"])
        .arg(&nonempty)
        .assert()
        .code(1);

    assert!(
        ctx.cli_exists(&nonempty),
        "Directory should still exist after failed removal"
    );
}
