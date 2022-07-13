use crate::cli::{
    fixtures::*,
    utils::{random_tenant, FAILURE_LINE},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::data::{Error, ErrorKind};
use predicates::prelude::*;
use rstest::*;
use serde_json::{json, Value};

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
fn should_support_copying_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("file");
    src.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("file2");

    // distant action copy {src} {dst}
    action_cmd
        .args(&["copy", src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    src.assert(predicate::path::exists());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
fn should_support_copying_nonempty_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let src = temp.child("dir");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("dir2");
    let dst_file = dst.child("file");

    // distant action copy {src} {dst}
    action_cmd
        .args(&["copy", src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    src_file.assert(predicate::path::exists());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("dir");
    let dst = temp.child("dir2");

    // distant action copy {src} {dst}
    action_cmd
        .args(&["copy", src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());

    src.assert(predicate::path::missing());
    dst.assert(predicate::path::missing());
}

#[rstest]
fn should_support_json_copying_file(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("file");
    src.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("file2");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "src": src.to_path_buf(),
            "dst": dst.to_path_buf(),
        },
    });

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", req))
        .assert()
        .success()
        .stderr("");

    let res: Value = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert_eq!(res, json!({}));

    src.assert(predicate::path::exists());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
fn should_support_json_copying_nonempty_directory(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let src = temp.child("dir");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("dir2");
    let dst_file = dst.child("file");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Copy {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
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

    src_file.assert(predicate::path::exists());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("dir");
    let dst = temp.child("dir2");

    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::Copy {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
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

    src.assert(predicate::path::missing());
    dst.assert(predicate::path::missing());
}
