//! Integration tests for the `distant fs exists` CLI subcommand.
//!
//! Tests checking whether a path exists on the filesystem.

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_output_true_if_exists(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create file
    let file = temp.child("file");
    file.touch().unwrap();

    // distant fs exists {path}
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("true\n");
}

#[rstest]
#[test_log::test]
fn should_output_false_if_not_exists(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    // distant fs exists {path}
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("false\n");
}

#[rstest]
#[test_log::test]
fn should_output_true_for_directory(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    // distant fs exists {path}
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(dir.to_str().unwrap())
        .assert()
        .success()
        .stdout("true\n");
}

#[rstest]
#[test_log::test]
fn should_always_exit_zero_for_false(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("nonexistent");

    // Even when path doesn't exist, exit code should be 0 (success)
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("false\n");
}
