use crate::cli::{
    fixtures::*,
    utils::{random_tenant, FAILURE_LINE},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind},
    Request, RequestData, Response, ResponseData,
};
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

#[rstest]
fn should_support_json_removing_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Remove {
            path: file.to_path_buf(),
            force: false,
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
    assert_eq!(res.payload[0], ResponseData::Ok);

    file.assert(predicate::path::missing());
}

#[rstest]
fn should_support_json_removing_empty_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Remove {
            path: dir.to_path_buf(),
            force: false,
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
    assert_eq!(res.payload[0], ResponseData::Ok);

    dir.assert(predicate::path::missing());
}

#[rstest]
fn should_support_json_removing_nonempty_directory_if_force_specified(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Remove {
            path: dir.to_path_buf(),
            force: true,
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
    assert_eq!(res.payload[0], ResponseData::Ok);

    dir.assert(predicate::path::missing());
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory so we fail to remove it
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Remove {
            path: dir.to_path_buf(),
            force: false,
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
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Error(Error {
                kind: ErrorKind::Other,
                ..
            })
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}
