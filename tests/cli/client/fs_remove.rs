use crate::cli::fixtures::*;
use crate::cli::utils::directory_not_empty_pred;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_support_removing_file(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    // distant action remove {path}
    ctx.new_assert_cmd(["fs", "remove"])
        .args([file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    file.assert(predicate::path::missing());
}

#[rstest]
#[test_log::test]
fn should_support_removing_empty_directory(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    // distant action remove {path}
    ctx.new_assert_cmd(["fs", "remove"])
        .args([dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::missing());
}

#[rstest]
#[test_log::test]
fn should_support_removing_nonempty_directory_if_force_specified(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    // distant action remove --force {path}
    ctx.new_assert_cmd(["fs", "remove"])
        .args(["--force", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::missing());
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    // distant action remove {path}
    ctx.new_assert_cmd(["fs", "remove"])
        .args([dir.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(directory_not_empty_pred());

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}
