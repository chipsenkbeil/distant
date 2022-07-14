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

const APPENDED_FILE_CONTENTS: &str = r#"
even more
file contents
"#;

#[rstest]
fn should_report_ok_when_done(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant action file-append {path} -- {contents}
    action_cmd
        .args(&[
            "file-append",
            file.to_str().unwrap(),
            "--",
            APPENDED_FILE_CONTENTS,
        ])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    // NOTE: We wait a little bit to give the OS time to fully write to file
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Because we're talking to a local server, we can verify locally
    file.assert(format!("{}{}", FILE_CONTENTS, APPENDED_FILE_CONTENTS));
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    // distant action file-append {path} -- {contents}
    action_cmd
        .args(&[
            "file-append",
            file.to_str().unwrap(),
            "--",
            APPENDED_FILE_CONTENTS,
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
    file.write_str(FILE_CONTENTS).unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::FileAppend {
            path: file.to_path_buf(),
            data: APPENDED_FILE_CONTENTS.as_bytes().to_vec(),
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

    // NOTE: We wait a little bit to give the OS time to fully write to file
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Because we're talking to a local server, we can verify locally
    file.assert(format!("{}{}", FILE_CONTENTS, APPENDED_FILE_CONTENTS));
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::FileAppend {
            path: file.to_path_buf(),
            data: APPENDED_FILE_CONTENTS.as_bytes().to_vec(),
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
