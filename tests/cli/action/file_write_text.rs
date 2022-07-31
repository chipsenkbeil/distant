use crate::cli::{fixtures::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use assert_fs::prelude::*;
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

    // NOTE: We wait a little bit to give the OS time to fully write to file
    std::thread::sleep(std::time::Duration::from_millis(100));

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
        .code(1)
        .stdout("")
        .stderr(FAILURE_LINE.clone());

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}
