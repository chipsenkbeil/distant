//! Integration tests for the `distant fs set-permissions` CLI subcommand.
//!
//! Tests setting file permissions with readonly keyword and error handling
//! for non-existent paths.

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
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_set_file_readonly_with_octal_mode(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms-octal");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file");
    ctx.cli_write(&path, "hello");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("0444")
        .arg(&path)
        .assert()
        .success();

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&path)
        .output()
        .expect("metadata failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Readonly: true"),
        "Expected 'Readonly: true' after setting 0444, got: {stdout}"
    );
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_set_file_permissions_with_symbolic_mode(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms-symbolic");
    ctx.cli_mkdir(&dir);
    let path = ctx.child_path(&dir, "test-file");
    ctx.cli_write(&path, "hello");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("u+rwx")
        .arg(&path)
        .assert()
        .success();

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&path)
        .output()
        .expect("metadata failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Readonly: false"),
        "Expected 'Readonly: false' after u+rwx, got: {stdout}"
    );
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_set_permissions_recursively(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("perms-recursive");
    ctx.cli_mkdir(&dir);
    let sub = ctx.child_path(&dir, "sub");
    ctx.cli_mkdir(&sub);
    let file_path = ctx.child_path(&sub, "nested-file");
    ctx.cli_write(&file_path, "content");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("--recursive")
        .arg("u+r,u-w,g-w,o-w")
        .arg(&dir)
        .assert()
        .success();

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&file_path)
        .output()
        .expect("metadata failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Readonly: true"),
        "Expected 'Readonly: true' for nested file, got: {stdout}"
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
