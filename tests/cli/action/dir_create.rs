use crate::cli::{fixtures::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_report_ok_when_done(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");

    // distant action dir-create {path}
    action_cmd
        .args(&["dir-create", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
fn should_support_creating_missing_parent_directories_if_specified(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir1").child("dir2");

    // distant action dir-create {path}
    action_cmd
        .args(&["dir-create", "--all", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("missing-dir").child("dir");

    // distant action dir-create {path}
    action_cmd
        .args(&["dir-create", dir.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(FAILURE_LINE.clone());

    dir.assert(predicate::path::missing());
}
