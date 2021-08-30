use crate::cli::{
    fixtures::*,
    utils::{random_tenant, regex_pred, FAILURE_LINE},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind, FileType},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
fn should_output_metadata_for_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant action metdata {path}
    action_cmd
        .args(&["metadata", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: file\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )))
        .stderr("");
}

#[rstest]
fn should_output_metadata_for_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    // distant action metadata {path}
    action_cmd
        .args(&["metadata", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: dir\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )))
        .stderr("");
}

#[rstest]
fn should_support_including_a_canonicalized_path(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    // distant action metdata --canonicalize {path}
    action_cmd
        .args(&["metadata", "--canonicalize", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: file\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )))
        .stderr("");
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    // distant action metadata {path}
    action_cmd
        .args(&["metadata", file.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}

#[rstest]
fn should_support_json_metadata_for_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.write_str(FILE_CONTENTS).unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Metadata {
            path: file.to_path_buf(),
            canonicalize: false,
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
            ResponseData::Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                readonly: false,
                ..
            },
        ),
        "Unexpected response: {:?}",
        res.payload[0],
    );
}

#[rstest]
fn should_support_json_metadata_for_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Metadata {
            path: dir.to_path_buf(),
            canonicalize: false,
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
            ResponseData::Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                readonly: false,
                ..
            },
        ),
        "Unexpected response: {:?}",
        res.payload[0],
    );
}

#[rstest]
fn should_support_json_metadata_for_including_a_canonicalized_path(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Metadata {
            path: file.to_path_buf(),
            canonicalize: true,
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
            ResponseData::Metadata {
                canonicalized_path: Some(_),
                file_type: FileType::File,
                readonly: false,
                ..
            },
        ),
        "Unexpected response: {:?}",
        res.payload[0],
    );
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Metadata {
            path: file.to_path_buf(),
            canonicalize: false,
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
}
