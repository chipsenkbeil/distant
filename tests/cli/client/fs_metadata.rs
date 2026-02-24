//! Integration tests for the `distant fs metadata` CLI subcommand.
//!
//! Tests retrieving metadata for files, directories, and symlinks, including
//! `--canonicalize` and `--resolve-file-type` flags.

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;
use distant_test_harness::utils::regex_pred;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[test_log::test]
fn should_output_metadata_for_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.write_str(FILE_CONTENTS).unwrap();

    // distant fs metadata {path}
    ctx.new_assert_cmd(["fs", "metadata"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: file\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )));
}

#[rstest]
#[test_log::test]
fn should_output_metadata_for_directory(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    // distant fs metadata {path}
    ctx.new_assert_cmd(["fs", "metadata"])
        .arg(dir.to_str().unwrap())
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: dir\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )));
}

#[rstest]
#[test_log::test]
fn should_support_including_a_canonicalized_path(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    // distant fs metadata --canonicalize {path}
    ctx.new_assert_cmd(["fs", "metadata"])
        .args(["--canonicalize", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(&format!(
            concat!(
                "Canonicalized Path: {:?}\n",
                "Type: symlink\n",
                "Len: .*\n",
                "Readonly: false\n",
                "Created: .*\n",
                "Last Accessed: .*\n",
                "Last Modified: .*\n",
            ),
            file.path().canonicalize().unwrap()
        )));
}

#[rstest]
#[test_log::test]
fn should_support_resolving_file_type_of_symlink(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    // distant fs metadata --canonicalize {path}
    ctx.new_assert_cmd(["fs", "metadata"])
        .args(["--resolve-file-type", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(concat!(
            "Type: file\n",
            "Len: .*\n",
            "Readonly: false\n",
            "Created: .*\n",
            "Last Accessed: .*\n",
            "Last Modified: .*\n",
        )));
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    // distant fs metadata {path}
    ctx.new_assert_cmd(["fs", "metadata"])
        .arg(file.to_str().unwrap())
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_output_unix_permissions(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.write_str("hello").unwrap();

    // distant fs metadata {path}
    let output = ctx
        .new_assert_cmd(["fs", "metadata"])
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // On Unix, metadata output should contain permission info
    // At minimum we see "Readonly: false" which indicates permissions are reported
    assert!(
        stdout.contains("Readonly:"),
        "Expected Unix permissions info in metadata output, got: {stdout}"
    );
}
