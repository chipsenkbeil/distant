use assert_fs::prelude::*;
use rstest::*;

use crate::common::fixtures::*;

#[rstest]
#[test_log::test]
fn should_output_true_if_exists(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create file
    let file = temp.child("file");
    file.touch().unwrap();

    // distant fs exists {path}
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("true\n");
}

#[rstest]
#[test_log::test]
fn should_output_false_if_not_exists(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    // distant fs exists {path}
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("false\n");
}
