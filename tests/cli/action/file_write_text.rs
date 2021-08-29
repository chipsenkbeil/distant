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
use rstest::*;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
fn should_report_ok_when_done(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    // distant action file-write-text {path} -- {contents}
    action_cmd
        .args(&[
            "file-write-text",
            file.to_str().unwrap(),
            "--",
            FILE_CONTENTS,
        ])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    // Because we're talking to a local server, we can verify locally
    file.assert(FILE_CONTENTS);
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    // distant action file-write {path} -- {contents}
    action_cmd
        .args(&[
            "file-write-text",
            file.to_str().unwrap(),
            "--",
            FILE_CONTENTS,
        ])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}

#[rstest]
fn should_support_json_output(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::FileWriteText {
            path: file.to_path_buf(),
            text: FILE_CONTENTS.to_string(),
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
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Because we're talking to a local server, we can verify locally
    file.assert(FILE_CONTENTS);
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::FileWriteText {
            path: file.to_path_buf(),
            text: FILE_CONTENTS.to_string(),
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
                kind: ErrorKind::NotFound,
                ..
            })
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}
