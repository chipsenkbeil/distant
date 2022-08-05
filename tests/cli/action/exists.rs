use crate::cli::fixtures::*;
use assert_cmd::Command;
use assert_fs::prelude::*;
use rstest::*;

#[rstest]
fn should_output_true_if_exists(mut action_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create file
    let file = temp.child("file");
    file.touch().unwrap();

    // distant action exists {path}
    action_cmd
        .args(&["exists", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("true\n")
        .stderr("");
}

#[rstest]
fn should_output_false_if_not_exists(mut action_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    // distant action exists {path}
    action_cmd
        .args(&["exists", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("false\n")
        .stderr("");
}
