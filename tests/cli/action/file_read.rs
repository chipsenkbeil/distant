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
#[test_log::test]
fn should_print_out_file_contents(mut action_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant action file-read {path}
    action_cmd
        .args(["file-read", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(format!("{}\n", FILE_CONTENTS))
        .stderr("");
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(mut action_cmd: CtxCommand<Command>) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    // distant action file-read {path}
    action_cmd
        .args(["file-read", file.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}
