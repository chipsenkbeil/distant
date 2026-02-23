use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

const FILE_CONTENTS: &str = indoc! {r#"
    some text
    on multiple lines
    that is a file's contents
"#};

#[rstest]
#[test_log::test]
fn should_print_out_file_contents(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant fs read {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args([file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(FILE_CONTENTS);
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    // distant fs read {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args([file.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());
}
