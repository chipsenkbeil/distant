//! Integration tests for the `distant fs set-permissions` CLI subcommand.
//!
//! Tests setting file permissions with readonly keyword and error handling
//! for non-existent paths. Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_set_readonly_and_verify_readable(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "perms-test.txt");
    ctx.cli_write(&path, "perms content");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("readonly")
        .arg(&path)
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&path)
        .assert()
        .success()
        .stdout("perms content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_fail_if_path_does_not_exist(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms-err");
    ctx.cli_mkdir(&dir);
    let missing = ctx.child_path(&dir, "nonexistent");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("0644")
        .arg(&missing)
        .assert()
        .failure();
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_set_file_readonly_with_octal_mode(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("hello").unwrap();

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("0400")
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    let meta = std::fs::metadata(file.path()).unwrap();
    assert!(meta.permissions().readonly(), "File should be readonly");
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_set_file_permissions_with_symbolic_mode(#[case] backend: Backend) {
    use std::os::unix::fs::PermissionsExt;

    use assert_fs::prelude::*;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("hello").unwrap();

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("u+rwx")
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    let mode = std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o700, 0o700, "Owner should have rwx, got {:o}", mode);
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_set_permissions_recursively(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let file = dir.child("nested-file");
    file.write_str("content").unwrap();

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("--recursive")
        .arg("u+r,u-w,g-w,o-w")
        .arg(dir.to_str().unwrap())
        .assert()
        .success();

    let meta = std::fs::metadata(file.path()).unwrap();
    assert!(
        meta.permissions().readonly(),
        "Nested file should be readonly"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_fail_on_invalid_mode_string(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms-invalid");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file.txt");
    ctx.cli_write(&path, "hello");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("xyz")
        .arg(&path)
        .assert()
        .failure();
}
