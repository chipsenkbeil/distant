use crate::cli::{fixtures::*, utils::random_tenant};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant_core::{Request, RequestData, Response, ResponseData};
use rstest::*;

#[rstest]
fn should_output_true_if_exists(mut action_cmd: Command) {
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
fn should_output_false_if_not_exists(mut action_cmd: Command) {
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

#[rstest]
fn should_support_json_true_if_exists(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create file
    let file = temp.child("file");
    file.touch().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Exists {
            path: file.to_path_buf(),
        }],
    };

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", serde_json::to_string(&req).unwrap()))
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert_eq!(res.payload[0], ResponseData::Exists { value: true });
}

#[rstest]
fn should_support_json_false_if_not_exists(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Exists {
            path: file.to_path_buf(),
        }],
    };

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", serde_json::to_string(&req).unwrap()))
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert_eq!(res.payload[0], ResponseData::Exists { value: false });
}
