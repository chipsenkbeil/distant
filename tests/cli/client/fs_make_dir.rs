//! Integration tests for the `distant fs make-dir` CLI subcommand.
//!
//! Tests creating directories, creating nested directories with `--all`,
//! and error handling when the parent directory is missing.
//! Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_report_ok_when_done(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mkdir");
    ctx.cli_mkdir(&dir);
    let new_dir = ctx.child_path(&dir, "new-dir");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(&new_dir)
        .assert()
        .success();

    assert!(
        ctx.cli_exists(&new_dir),
        "Directory should be created (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_creating_missing_parent_directories_if_specified(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mkdir-all");
    ctx.cli_mkdir(&dir);
    let nested = ctx.child_path(&ctx.child_path(&dir, "dir1"), "dir2");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", &nested])
        .assert()
        .success();

    assert!(
        ctx.cli_exists(&nested),
        "Nested directory should be created (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mkdir-err");
    // Do NOT create the parent — so the child dir creation fails
    let nested = ctx.child_path(&ctx.child_path(&dir, "missing-dir"), "dir");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(&nested)
        .assert()
        .code(1);
}

/// Docker is excluded because its tar-based create_dir implementation
/// succeeds on existing directories (idempotent behavior).
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_fail_when_already_exists(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mkdir-exists");
    ctx.cli_mkdir(&dir);
    let existing = ctx.child_path(&dir, "existing-dir");
    ctx.cli_mkdir(&existing);

    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(&existing)
        .assert()
        .failure();
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_succeed_when_already_exists_with_all(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mkdir-exists-all");
    ctx.cli_mkdir(&dir);
    let existing = ctx.child_path(&dir, "existing-dir");
    ctx.cli_mkdir(&existing);

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", &existing])
        .assert()
        .success();
}
