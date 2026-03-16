//! Integration tests for the `distant copy` CLI subcommand.
//!
//! Tests local↔remote file transfers including single files, directories,
//! error cases, and edge cases like empty files and binary content.

use std::time::Duration;

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

const FILE_CONTENTS: &str = "some text\non multiple lines\nthat is a file's contents\n";
const WRITE_DELAY: Duration = Duration::from_millis(100);

#[rstest]
#[test_log::test]
fn should_upload_single_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create local source file
    let local_src = temp.child("local.txt");
    local_src.write_str(FILE_CONTENTS).unwrap();

    // Remote destination (on the local server, but accessed via protocol)
    let remote_dst = temp.child("remote.txt");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    // distant copy ./local.txt :/remote.txt
    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    // Verify the remote file was created with correct content
    remote_dst.assert(FILE_CONTENTS);
}

#[rstest]
#[test_log::test]
fn should_download_single_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create "remote" file using fs write
    let remote_src = temp.child("remote.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_src.to_str().unwrap()])
        .write_stdin(FILE_CONTENTS)
        .assert()
        .success();

    std::thread::sleep(WRITE_DELAY);

    // Local destination
    let local_dst = temp.child("local.txt");
    let remote_path = format!(":{}", remote_src.to_str().unwrap());

    // distant copy :/remote.txt ./local.txt
    ctx.new_assert_cmd(["copy"])
        .args([&remote_path, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert(FILE_CONTENTS);
}

#[rstest]
#[test_log::test]
fn should_upload_directory_recursively(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create local directory tree
    let local_dir = temp.child("src_dir");
    local_dir.create_dir_all().unwrap();
    let file_a = local_dir.child("a.txt");
    file_a.write_str("content a").unwrap();
    let sub_dir = local_dir.child("sub");
    sub_dir.create_dir_all().unwrap();
    let file_b = sub_dir.child("b.txt");
    file_b.write_str("content b").unwrap();

    // Remote destination
    let remote_dir = temp.child("dst_dir");
    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    // distant copy -r ./src_dir :/dst_dir
    ctx.new_assert_cmd(["copy"])
        .args(["-r", local_dir.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    // Verify remote directory structure
    remote_dir.child("a.txt").assert("content a");
    remote_dir.child("sub").child("b.txt").assert("content b");
}

#[rstest]
#[test_log::test]
fn should_download_directory_recursively(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create "remote" directory structure using fs commands
    let remote_dir = temp.child("remote_dir");
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([remote_dir.to_str().unwrap()])
        .assert()
        .success();

    let remote_sub = remote_dir.child("sub");
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([remote_sub.to_str().unwrap()])
        .assert()
        .success();

    let remote_file_a = remote_dir.child("a.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_file_a.to_str().unwrap()])
        .write_stdin("content a")
        .assert()
        .success();

    let remote_file_b = remote_sub.child("b.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_file_b.to_str().unwrap()])
        .write_stdin("content b")
        .assert()
        .success();

    std::thread::sleep(WRITE_DELAY);

    // Local destination
    let local_dir = temp.child("local_dir");
    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    // distant copy -r :/remote_dir ./local_dir
    ctx.new_assert_cmd(["copy"])
        .args(["-r", &remote_path, local_dir.to_str().unwrap()])
        .assert()
        .success();

    // Verify local directory structure
    local_dir.child("a.txt").assert("content a");
    local_dir.child("sub").child("b.txt").assert("content b");
}

#[rstest]
#[test_log::test]
fn should_error_when_both_paths_are_local(ctx: ManagerCtx) {
    ctx.new_assert_cmd(["copy"])
        .args(["./local_a", "./local_b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Both paths are local"));
}

#[rstest]
#[test_log::test]
fn should_error_when_both_paths_are_remote(ctx: ManagerCtx) {
    ctx.new_assert_cmd(["copy"])
        .args([":/remote_a", ":/remote_b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Both paths are remote"));
}

#[rstest]
#[test_log::test]
fn should_error_when_directory_without_recursive_flag(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let local_dir = temp.child("a_dir");
    local_dir.create_dir_all().unwrap();

    let remote_path = format!(":{}", temp.child("dst").to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_dir.to_str().unwrap(), &remote_path])
        .assert()
        .code(1)
        .stderr(predicates::str::contains(
            "is a directory (use -r to copy recursively)",
        ));
}

#[rstest]
#[test_log::test]
fn should_error_when_source_not_found(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let missing = temp.child("nonexistent.txt");
    let remote_path = format!(":{}", temp.child("dst.txt").to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([missing.to_str().unwrap(), &remote_path])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Failed to read"));
}

#[rstest]
#[test_log::test]
fn should_upload_into_existing_directory(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create local file
    let local_file = temp.child("myfile.txt");
    local_file.write_str("hello").unwrap();

    // Create remote directory
    let remote_dir = temp.child("existing_dir");
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([remote_dir.to_str().unwrap()])
        .assert()
        .success();

    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    // distant copy ./myfile.txt :/existing_dir
    // Should place file inside the directory as existing_dir/myfile.txt
    ctx.new_assert_cmd(["copy"])
        .args([local_file.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dir.child("myfile.txt").assert("hello");
}

#[rstest]
#[test_log::test]
fn should_handle_empty_file(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Upload empty file
    let local_src = temp.child("empty.txt");
    local_src.write_str("").unwrap();

    let remote_dst = temp.child("remote_empty.txt");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dst.assert("");

    // Download it back
    let local_dst = temp.child("downloaded_empty.txt");
    let remote_path2 = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert("");
}

#[rstest]
#[test_log::test]
fn should_preserve_binary_content(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create a file with non-UTF8 binary content
    let binary_data: Vec<u8> = (0..=255).collect();
    let local_src = temp.child("binary.bin");
    local_src.write_binary(&binary_data).unwrap();

    let remote_dst = temp.child("remote_binary.bin");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    // Upload
    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dst.assert(predicate::path::eq_file(local_src.path()));

    // Download back
    let local_roundtrip = temp.child("roundtrip.bin");
    let remote_path2 = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_roundtrip.to_str().unwrap()])
        .assert()
        .success();

    local_roundtrip.assert(predicate::path::eq_file(local_src.path()));
}

#[rstest]
#[test_log::test]
fn should_overwrite_existing_destination(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create initial remote file
    let remote_file = temp.child("target.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_file.to_str().unwrap()])
        .write_stdin("old content")
        .assert()
        .success();

    std::thread::sleep(WRITE_DELAY);

    // Upload new content to the same path
    let local_src = temp.child("new.txt");
    local_src.write_str("new content").unwrap();

    let remote_path = format!(":{}", remote_file.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_file.assert("new content");
}

#[rstest]
#[test_log::test]
fn should_upload_to_server_cwd_with_bare_colon(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("bare_colon.txt");
    local_src.write_str("bare colon content").unwrap();

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), ":"])
        .assert()
        .success();

    // The server CWD is the test runner's CWD (workspace root),
    // so the file lands at `current_dir/bare_colon.txt`.
    let landed = std::env::current_dir().unwrap().join("bare_colon.txt");
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        "bare colon content"
    );
    std::fs::remove_file(&landed).unwrap();
}
