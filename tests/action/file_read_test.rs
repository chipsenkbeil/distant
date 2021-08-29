use crate::fixtures::*;
use assert_fs::prelude::*;
use distant_core::{
    data::{Error, ErrorKind},
    Response, ResponseData,
};
use rstest::*;

#[rstest]
fn should_print_out_file_contents(ctx: DistantServerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some\ntext\ncontent").unwrap();

    ctx.new_cmd("action")
        .args(&["file-read", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout("some\ntext\ncontent\n")
        .stderr("");
}

#[rstest]
fn should_support_json_output(ctx: DistantServerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some\ntext\ncontent").unwrap();

    let cmd = ctx
        .new_cmd("action")
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
fn yield_an_error_when_fails(ctx: DistantServerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    let cmd = ctx
        .new_cmd("action")
        .args(&["--format", "json"])
        .args(&["file-read", file.to_str().unwrap()])
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
