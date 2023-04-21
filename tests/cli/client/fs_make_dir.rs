use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_report_ok_when_done(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test_log::test]
fn should_support_creating_missing_parent_directories_if_specified(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir1").child("dir2");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("missing-dir").child("dir");

    // distant action dir-create {path}
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("No such file or directory"));

    dir.assert(predicate::path::missing());
}
