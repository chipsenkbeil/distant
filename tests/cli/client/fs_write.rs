//! Integration tests for the `distant fs write` CLI subcommand.
//!
//! Tests writing content to files via stdin and argument input, with optional
//! append mode, and error handling for missing parent directories.
//! Runs against Host, SSH, and Docker backends.

use std::io::Write as _;
use std::process::Stdio;

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_writing_stdin_to_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("write-stdin");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");

    let mut child = ctx
        .new_std_cmd(["fs", "write"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn fs write");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"written via stdin")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "fs write should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let contents = ctx.cli_read(&path);
    assert_eq!(contents, "written via stdin");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_appending_stdin_to_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("write-append");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");
    ctx.cli_write(&path, "initial content");

    ctx.new_assert_cmd(["fs", "write"])
        .args(["--append", &path])
        .write_stdin(" appended")
        .assert()
        .success()
        .stdout("");

    let contents = ctx.cli_read(&path);
    assert_eq!(contents, "initial content appended");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_writing_argument_to_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("write-arg");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");

    ctx.new_assert_cmd(["fs", "write"])
        .args([&path, "--"])
        .arg("arg content")
        .assert()
        .success()
        .stdout("");

    let contents = ctx.cli_read(&path);
    assert_eq!(contents, "arg content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_appending_argument_to_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("write-append-arg");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");
    ctx.cli_write(&path, "base");

    ctx.new_assert_cmd(["fs", "write"])
        .args(["--append", &path, "--"])
        .arg(" extra")
        .assert()
        .success()
        .stdout("");

    let contents = ctx.cli_read(&path);
    assert_eq!(contents, "base extra");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_overwrite_existing_file_content(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("write-overwrite");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");
    ctx.cli_write(&path, "initial content");

    ctx.new_assert_cmd(["fs", "write"])
        .arg(&path)
        .write_stdin("replaced content")
        .assert()
        .success();

    let contents = ctx.cli_read(&path);
    assert_eq!(contents, "replaced content");
}
