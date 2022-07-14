use crate::cli::{fixtures::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_support_removing_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    // distant action remove {path}
    action_cmd
        .args(&["remove", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    file.assert(predicate::path::missing());
}

#[rstest]
fn should_support_removing_empty_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    // distant action remove {path}
    action_cmd
        .args(&["remove", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::missing());
}

#[rstest]
fn should_support_removing_nonempty_directory_if_force_specified(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    // distant action remove --force {path}
    action_cmd
        .args(&["remove", "--force", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    dir.assert(predicate::path::missing());
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    // distant action remove {path}
    action_cmd
        .args(&["remove", dir.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}
