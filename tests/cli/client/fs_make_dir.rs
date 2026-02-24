//! Integration tests for the `distant fs make-dir` CLI subcommand.
//!
//! Tests creating directories, creating nested directories with `--all`,
//! and error handling when the parent directory is missing.

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_report_ok_when_done(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test_log::test]
fn should_support_creating_missing_parent_directories_if_specified(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir1").child("dir2");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("missing-dir").child("dir");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());

    dir.assert(predicate::path::missing());
}

#[rstest]
#[test_log::test]
fn should_fail_when_already_exists(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("existing-dir");
    dir.create_dir_all().unwrap();

    // Without --all, creating an existing directory should fail
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir.to_str().unwrap()])
        .assert()
        .failure();
}

#[rstest]
#[test_log::test]
fn should_succeed_when_already_exists_with_all(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("existing-dir");
    dir.create_dir_all().unwrap();

    // With --all, creating an existing directory should succeed silently
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("");
}
