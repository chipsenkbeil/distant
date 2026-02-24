//! Integration tests for the `distant fs read` CLI subcommand when used on directories.
//!
//! Tests listing directory contents with various options (depth, absolute paths,
//! include root) and error handling for missing directories.

use std::path::Path;

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;
use distant_test_harness::utils::regex_pred;

/// Creates a directory in the form
///
/// $TEMP/
/// $TEMP/dir1/
/// $TEMP/dir1/dira/
/// $TEMP/dir1/dirb/
/// $TEMP/dir1/dirb/file1
/// $TEMP/dir1/file1
/// $TEMP/dir1/file2
/// $TEMP/dir2/
/// $TEMP/dir2/dira/
/// $TEMP/dir2/dirb/
/// $TEMP/dir2/dirb/file1
/// $TEMP/dir2/file1
/// $TEMP/dir2/file2
/// $TEMP/file1
/// $TEMP/file2
fn make_directory() -> assert_fs::TempDir {
    let temp = assert_fs::TempDir::new().unwrap();

    // $TEMP/file1
    // $TEMP/file2
    temp.child("file1").touch().unwrap();
    temp.child("file2").touch().unwrap();

    // $TEMP/dir1/
    // $TEMP/dir1/file1
    // $TEMP/dir1/file2
    let dir1 = temp.child("dir1");
    dir1.create_dir_all().unwrap();
    dir1.child("file1").touch().unwrap();
    dir1.child("file2").touch().unwrap();

    // $TEMP/dir1/dira/
    let dir1_dira = dir1.child("dira");
    dir1_dira.create_dir_all().unwrap();

    // $TEMP/dir1/dirb/
    // $TEMP/dir1/dirb/file1
    let dir1_dirb = dir1.child("dirb");
    dir1_dirb.create_dir_all().unwrap();
    dir1_dirb.child("file1").touch().unwrap();

    // $TEMP/dir2/
    // $TEMP/dir2/file1
    // $TEMP/dir2/file2
    let dir2 = temp.child("dir2");
    dir2.create_dir_all().unwrap();
    dir2.child("file1").touch().unwrap();
    dir2.child("file2").touch().unwrap();

    // $TEMP/dir2/dira/
    let dir2_dira = dir2.child("dira");
    dir2_dira.create_dir_all().unwrap();

    // $TEMP/dir2/dirb/
    // $TEMP/dir2/dirb/file1
    let dir2_dirb = dir2.child("dirb");
    dir2_dirb.create_dir_all().unwrap();
    dir2_dirb.child("file1").touch().unwrap();

    temp
}

fn regex_stdout<'a>(lines: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut s = String::new();

    s.push('^');
    for (ty, path) in lines {
        s.push_str(&regex_line(ty, path));
    }
    s.push('$');

    s
}

fn regex_line(ty: &str, path: &str) -> String {
    format!(r"\s*{ty}\s+{path}\s*[\r\n]*")
}

#[rstest]
#[test_log::test]
fn should_print_immediate_files_and_directories_by_default(ctx: ManagerCtx) {
    let temp = make_directory();

    let expected = regex_pred(&regex_stdout(vec![
        ("<DIR>", "dir1"),
        ("<DIR>", "dir2"),
        ("", "file1"),
        ("", "file2"),
    ]));

    // distant fs read {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args([temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected);
}

#[rstest]
#[test_log::test]
fn should_use_absolute_paths_if_specified(ctx: ManagerCtx) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so the absolute path
    //       provided is our canonicalized root path prepended
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let expected = regex_pred(&regex_stdout(vec![
        ("<DIR>", root_path.join("dir1").to_str().unwrap()),
        ("<DIR>", root_path.join("dir2").to_str().unwrap()),
        ("", root_path.join("file1").to_str().unwrap()),
        ("", root_path.join("file2").to_str().unwrap()),
    ]));

    // distant fs read --absolute {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args(["--absolute", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected);
}

#[rstest]
#[test_log::test]
fn should_print_all_files_and_directories_if_depth_is_0(ctx: ManagerCtx) {
    let temp = make_directory();

    let expected = regex_pred(&regex_stdout(vec![
        ("<DIR>", Path::new("dir1").to_str().unwrap()),
        ("<DIR>", Path::new("dir1").join("dira").to_str().unwrap()),
        ("<DIR>", Path::new("dir1").join("dirb").to_str().unwrap()),
        (
            "",
            Path::new("dir1")
                .join("dirb")
                .join("file1")
                .to_str()
                .unwrap(),
        ),
        ("", Path::new("dir1").join("file1").to_str().unwrap()),
        ("", Path::new("dir1").join("file2").to_str().unwrap()),
        ("<DIR>", Path::new("dir2").to_str().unwrap()),
        ("<DIR>", Path::new("dir2").join("dira").to_str().unwrap()),
        ("<DIR>", Path::new("dir2").join("dirb").to_str().unwrap()),
        (
            "",
            Path::new("dir2")
                .join("dirb")
                .join("file1")
                .to_str()
                .unwrap(),
        ),
        ("", Path::new("dir2").join("file1").to_str().unwrap()),
        ("", Path::new("dir2").join("file2").to_str().unwrap()),
        ("", Path::new("file1").to_str().unwrap()),
        ("", Path::new("file2").to_str().unwrap()),
    ]));

    // distant fs read --depth 0 {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args(["--depth", "0", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected);
}

#[rstest]
#[test_log::test]
fn should_include_root_directory_if_specified(ctx: ManagerCtx) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so yielded entry
    //       is the canonicalized version
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let expected = regex_pred(&regex_stdout(vec![
        ("<DIR>", root_path.to_str().unwrap()),
        ("<DIR>", "dir1"),
        ("<DIR>", "dir2"),
        ("", "file1"),
        ("", "file2"),
    ]));

    // distant fs read --include-root {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args(["--include-root", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected);
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    let temp = make_directory();
    let dir = temp.child("missing-dir");

    // distant fs read {path}
    ctx.new_assert_cmd(["fs", "read"])
        .args([dir.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::is_empty().not());
}

#[rstest]
#[test_log::test]
fn should_support_canonicalize_flag(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let target_dir = temp.child("target_dir");
    target_dir.create_dir_all().unwrap();
    target_dir.child("file1").touch().unwrap();

    // Create a symlink pointing to target_dir
    let link = temp.child("link");
    link.symlink_to_dir(target_dir.path()).unwrap();

    // distant fs read --canonicalize --absolute {link}
    // --canonicalize resolves symlinks, --absolute shows full paths
    let output = ctx
        .new_assert_cmd(["fs", "read"])
        .args(["--canonicalize", "--absolute", link.to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // Canonicalized output should resolve the symlink and show the real target path
    let canonical_target = target_dir.path().canonicalize().unwrap();
    assert!(
        stdout.contains(canonical_target.join("file1").to_str().unwrap()),
        "Expected canonicalized path through symlink to resolve to real target, got: {stdout}"
    );
}
