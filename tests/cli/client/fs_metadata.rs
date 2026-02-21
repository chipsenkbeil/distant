use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use crate::common::fixtures::*;
use crate::common::utils::regex_pred;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[test_log::test]
fn should_output_metadata_for_file(ctx: DistantManagerCtx) {
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
fn should_output_metadata_for_directory(ctx: DistantManagerCtx) {
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

// NOTE: Ignoring on windows because SSH doesn't properly canonicalize paths to resolve symlinks!
#[rstest]
#[test_log::test]
#[cfg_attr(windows, ignore)]
fn should_support_including_a_canonicalized_path(ctx: DistantManagerCtx) {
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
fn should_support_resolving_file_type_of_symlink(ctx: DistantManagerCtx) {
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
fn yield_an_error_when_fails(ctx: DistantManagerCtx) {
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
