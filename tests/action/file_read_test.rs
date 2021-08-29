use crate::{fixtures::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind},
    Response, ResponseData,
};
use rstest::*;

#[rstest]
fn should_print_out_file_contents(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some\ntext\ncontent").unwrap();

    // distant action file-read {path}
    action_cmd
        .args(&["file-read", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("some\ntext\ncontent\n")
        .stderr("");
}

#[rstest]
fn should_support_json_output(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some\ntext\ncontent").unwrap();

    // distant action --format json file-read {path}
    let cmd = action_cmd
        .args(&["--format", "json"])
        .args(&["file-read", file.to_str().unwrap()])
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert_eq!(
        res.payload[0],
        ResponseData::Blob {
            data: b"some\ntext\ncontent".to_vec()
        }
    );
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    // distant action file-read {path}
    action_cmd
        .args(&["file-read", file.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    // distant action --format json file-read {path}
    let cmd = action_cmd
        .args(&["--format", "json"])
        .args(&["file-read", file.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Error(Error {
                kind: ErrorKind::NotFound,
                ..
            })
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}
