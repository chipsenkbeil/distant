//! Integration tests for the `distant fs set-permissions` CLI subcommand.
//!
//! Tests setting file permissions with octal mode, symbolic mode, recursive mode,
//! and error handling for non-existent paths.

#![allow(unexpected_cfgs)]

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_set_file_readonly_with_octal_mode(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("hello").unwrap();

    // distant fs set-permissions 0400 {path}
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
#[test_log::test]
fn should_set_file_permissions_with_symbolic_mode(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("hello").unwrap();

    // distant fs set-permissions u+rwx {path}
    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("u+rwx")
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o700, 0o700, "Owner should have rwx, got {:o}", mode);
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_set_permissions_recursively(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let file = dir.child("nested-file");
    file.write_str("content").unwrap();

    // Use u+r,u-w,g-w,o-w to make readonly without removing directory traversal bits
    // (0444 on files, directories keep their exec bits for traversal)
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
#[test_log::test]
fn should_fail_if_path_does_not_exist(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let missing = temp.child("nonexistent");

    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("0644")
        .arg(missing.to_str().unwrap())
        .assert()
        .failure();
}
