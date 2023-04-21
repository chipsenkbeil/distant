use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::*;
use rstest::*;

const FILE_CONTENTS: &str = indoc! {r#"
    some text
    on multiple lines
    that is a file's contents
"#};

const APPENDED_FILE_CONTENTS: &str = indoc! {r#"
    even more
    file contents
"#};

#[rstest]
#[test_log::test]
fn should_support_writing_stdin_to_file(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    // distant action file-write {path} -- {contents}
    ctx.new_assert_cmd(["fs", "write"])
        .args([file.to_str().unwrap()])
        .write_stdin(FILE_CONTENTS)
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
#[test_log::test]
fn should_support_appending_stdin_to_file(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant action file-write {path} -- {contents}
    ctx.new_assert_cmd(["fs", "write"])
        .args(["--append", file.to_str().unwrap()])
        .write_stdin(APPENDED_FILE_CONTENTS)
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
#[test_log::test]
fn should_support_writing_argument_to_file(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    // distant action file-write {path} -- {contents}
    ctx.new_assert_cmd(["fs", "write"])
        .args([file.to_str().unwrap(), "--"])
        .arg(FILE_CONTENTS)
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
#[test_log::test]
fn should_support_appending_argument_to_file(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant action file-write {path} -- {contents}
    ctx.new_assert_cmd(["fs", "write"])
        .args(["--append", file.to_str().unwrap(), "--"])
        .arg(APPENDED_FILE_CONTENTS)
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
#[test_log::test]
fn yield_an_error_when_fails(ctx: DistantManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    // distant action file-write {path} -- {contents}
    ctx.new_assert_cmd(["fs", "write"])
        .args([file.to_str().unwrap(), "--"])
        .arg(FILE_CONTENTS)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}
