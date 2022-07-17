use crate::cli::{
    fixtures::*,
    utils::{regex_pred, FAILURE_LINE},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use rstest::*;
use std::path::Path;

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
    format!(r"\s*{ty}\s+{path}\s*\n")
}

#[rstest]
fn should_print_immediate_files_and_directories_by_default(mut action_cmd: Command) {
    let temp = make_directory();

    let expected = regex_pred(&regex_stdout(vec![
        ("<DIR>", "dir1"),
        ("<DIR>", "dir2"),
        ("", "file1"),
        ("", "file2"),
    ]));

    // distant action dir-read {path}
    action_cmd
        .args(&["dir-read", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected)
        .stderr("");
}

#[rstest]
fn should_use_absolute_paths_if_specified(mut action_cmd: Command) {
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

    // distant action dir-read --absolute {path}
    action_cmd
        .args(&["dir-read", "--absolute", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected)
        .stderr("");
}

#[rstest]
fn should_print_all_files_and_directories_if_depth_is_0(mut action_cmd: Command) {
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

    // distant action dir-read --depth 0 {path}
    action_cmd
        .args(&["dir-read", "--depth", "0", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected)
        .stderr("");
}

#[rstest]
fn should_include_root_directory_if_specified(mut action_cmd: Command) {
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

    // distant action dir-read --include-root {path}
    action_cmd
        .args(&["dir-read", "--include-root", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(expected)
        .stderr("");
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    let temp = make_directory();
    let dir = temp.child("missing-dir");

    // distant action dir-read {path}
    action_cmd
        .args(&["dir-read", dir.to_str().unwrap()])
        .assert()
        .code(ExitCode::Software.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}
