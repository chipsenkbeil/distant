//! Integration tests for the `distant fs read` CLI subcommand when used on files.
//!
//! Tests reading file contents to stdout and error handling for missing files.
//! Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_print_out_file_contents(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("read-file");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");
    ctx.cli_write(
        &path,
        "some text\non multiple lines\nthat is a file's contents\n",
    );

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&path)
        .assert()
        .success()
        .stdout("some text\non multiple lines\nthat is a file's contents\n");
}

/// Docker is excluded because its tar-based read_file implementation
/// may handle missing files differently from Host/SSH backends.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("read-file-err");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "missing-file");

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&path)
        .assert()
        .code(1)
        .stdout("");
}
