use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[test_log::test]
fn should_support_renaming_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("file");
    src.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("file2");

    // distant action rename {src} {dst}
    ctx.new_assert_cmd(["fs", "rename"])
        .args([src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .success()
        .stdout("");

    src.assert(predicate::path::missing());
    dst.assert(FILE_CONTENTS);
}

#[rstest]
#[test_log::test]
fn should_support_renaming_nonempty_directory(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let src = temp.child("dir");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("dir2");
    let dst_file = dst.child("file");

    // distant action rename {src} {dst}
    ctx.new_assert_cmd(["fs", "rename"])
        .args([src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .success()
        .stdout("");

    src.assert(predicate::path::missing());
    src_file.assert(predicate::path::missing());

    dst.assert(predicate::path::is_dir());
    dst_file.assert(FILE_CONTENTS);
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("dir");
    let dst = temp.child("dir2");

    // distant action rename {src} {dst}
    ctx.new_assert_cmd(["fs", "rename"])
        .args([src.to_str().unwrap(), dst.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());

    src.assert(predicate::path::missing());
    dst.assert(predicate::path::missing());
}
