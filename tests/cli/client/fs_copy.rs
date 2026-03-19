//! Integration tests for the `distant fs copy` CLI subcommand.
//!
//! Tests copying files, copying non-empty directories, and error handling
//! when the source does not exist.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_copying_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("fs-copy");
    ctx.cli_mkdir(&dir);
    let src = ctx.child_path(&dir, "copy-src.txt");
    let dst = ctx.child_path(&dir, "copy-dst.txt");
    ctx.cli_write(&src, "copy content");

    ctx.new_assert_cmd(["fs", "copy"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .success();

    let contents = ctx.cli_read(&dst);
    assert_eq!(contents, "copy content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_copying_nonempty_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("fs-copy-dir");
    ctx.cli_mkdir(&dir);

    let src_dir = ctx.child_path(&dir, "src");
    ctx.cli_mkdir(&src_dir);
    let src_file = ctx.child_path(&src_dir, "file.txt");
    ctx.cli_write(&src_file, "dir copy content");

    let dst_dir = ctx.child_path(&dir, "dst");

    ctx.new_assert_cmd(["fs", "copy"])
        .arg(&src_dir)
        .arg(&dst_dir)
        .assert()
        .success();

    let dst_file = ctx.child_path(&dst_dir, "file.txt");
    let contents = ctx.cli_read(&dst_file);
    assert_eq!(contents, "dir copy content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("fs-copy-err");
    ctx.cli_mkdir(&dir);
    let src = ctx.child_path(&dir, "nonexistent");
    let dst = ctx.child_path(&dir, "dst");

    ctx.new_assert_cmd(["fs", "copy"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .code(1);
}
