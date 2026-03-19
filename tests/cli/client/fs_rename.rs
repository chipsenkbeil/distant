//! Integration tests for the `distant fs rename` CLI subcommand.
//!
//! Tests renaming files, renaming non-empty directories, and error handling
//! when the source does not exist.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_renaming_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("rename");
    ctx.cli_mkdir(&dir);
    let src = ctx.child_path(&dir, "rename-src.txt");
    let dst = ctx.child_path(&dir, "rename-dst.txt");
    ctx.cli_write(&src, "rename content");

    ctx.new_assert_cmd(["fs", "rename"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .success();

    assert!(
        !ctx.cli_exists(&src),
        "Source should no longer exist (verified via CLI)"
    );
    assert!(
        ctx.cli_exists(&dst),
        "Destination should exist (verified via CLI)"
    );
    let contents = ctx.cli_read(&dst);
    assert_eq!(contents, "rename content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_renaming_nonempty_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("rename-dir");
    ctx.cli_mkdir(&dir);

    let src_dir = ctx.child_path(&dir, "src");
    ctx.cli_mkdir(&src_dir);
    ctx.cli_write(&ctx.child_path(&src_dir, "file.txt"), "dir rename content");

    let dst_dir = ctx.child_path(&dir, "dst");

    ctx.new_assert_cmd(["fs", "rename"])
        .arg(&src_dir)
        .arg(&dst_dir)
        .assert()
        .success();

    assert!(
        !ctx.cli_exists(&src_dir),
        "Source directory should no longer exist"
    );
    assert!(
        ctx.cli_exists(&dst_dir),
        "Destination directory should exist"
    );
    let contents = ctx.cli_read(&ctx.child_path(&dst_dir, "file.txt"));
    assert_eq!(contents, "dir rename content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("rename-err");
    ctx.cli_mkdir(&dir);
    let src = ctx.child_path(&dir, "nonexistent");
    let dst = ctx.child_path(&dir, "dst");

    ctx.new_assert_cmd(["fs", "rename"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .code(1);
}
