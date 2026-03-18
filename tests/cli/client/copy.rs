//! Integration tests for the `distant copy` CLI subcommand.
//!
//! Tests local-to-remote and remote-to-local file transfers including single
//! files, directories, error cases, and edge cases like empty files and binary
//! content. This is distinct from `distant fs copy` which copies within the
//! remote filesystem. Host-only since it requires local filesystem access.

use std::time::Duration;

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

const FILE_CONTENTS: &str = "some text\non multiple lines\nthat is a file's contents\n";
const WRITE_DELAY: Duration = Duration::from_millis(100);

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_upload_single_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("local.txt");
    local_src.write_str(FILE_CONTENTS).unwrap();

    let remote_dst = temp.child("remote.txt");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dst.assert(FILE_CONTENTS);
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_download_single_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let remote_src = temp.child("remote.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_src.to_str().unwrap()])
        .write_stdin(FILE_CONTENTS)
        .assert()
        .success();

    std::thread::sleep(WRITE_DELAY);

    let local_dst = temp.child("local.txt");
    let remote_path = format!(":{}", remote_src.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert(FILE_CONTENTS);
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_upload_directory_recursively(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_dir = temp.child("src_dir");
    local_dir.create_dir_all().unwrap();
    let file_a = local_dir.child("a.txt");
    file_a.write_str("content a").unwrap();
    let sub_dir = local_dir.child("sub");
    sub_dir.create_dir_all().unwrap();
    let file_b = sub_dir.child("b.txt");
    file_b.write_str("content b").unwrap();

    let remote_dir = temp.child("dst_dir");
    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args(["-r", local_dir.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dir.child("a.txt").assert("content a");
    remote_dir.child("sub").child("b.txt").assert("content b");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_download_directory_recursively(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

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

    let local_dir = temp.child("local_dir");
    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args(["-r", &remote_path, local_dir.to_str().unwrap()])
        .assert()
        .success();

    local_dir.child("a.txt").assert("content a");
    local_dir.child("sub").child("b.txt").assert("content b");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_error_when_both_paths_are_local(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    ctx.new_assert_cmd(["copy"])
        .args(["./local_a", "./local_b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Both paths are local"));
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_error_when_both_paths_are_remote(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    ctx.new_assert_cmd(["copy"])
        .args([":/remote_a", ":/remote_b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Both paths are remote"));
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_error_when_directory_without_recursive_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
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
#[case::host(Backend::Host)]
#[test_log::test]
fn should_error_when_source_not_found(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
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
#[case::host(Backend::Host)]
#[test_log::test]
fn should_upload_into_existing_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_file = temp.child("myfile.txt");
    local_file.write_str("hello").unwrap();

    let remote_dir = temp.child("existing_dir");
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([remote_dir.to_str().unwrap()])
        .assert()
        .success();

    let remote_path = format!(":{}", remote_dir.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_file.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dir.child("myfile.txt").assert("hello");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_handle_empty_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("empty.txt");
    local_src.write_str("").unwrap();

    let remote_dst = temp.child("remote_empty.txt");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dst.assert("");

    let local_dst = temp.child("downloaded_empty.txt");
    let remote_path2 = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert("");
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_preserve_binary_content(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let binary_data: Vec<u8> = (0..=255).collect();
    let local_src = temp.child("binary.bin");
    local_src.write_binary(&binary_data).unwrap();

    let remote_dst = temp.child("remote_binary.bin");
    let remote_path = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    remote_dst.assert(predicate::path::eq_file(local_src.path()));

    let local_roundtrip = temp.child("roundtrip.bin");
    let remote_path2 = format!(":{}", remote_dst.to_str().unwrap());

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_roundtrip.to_str().unwrap()])
        .assert()
        .success();

    local_roundtrip.assert(predicate::path::eq_file(local_src.path()));
}

#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn should_overwrite_existing_destination(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let remote_file = temp.child("target.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([remote_file.to_str().unwrap()])
        .write_stdin("old content")
        .assert()
        .success();

    std::thread::sleep(WRITE_DELAY);

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
#[case::host(Backend::Host)]
#[test_log::test]
fn should_upload_to_server_cwd_with_bare_colon(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("bare_colon.txt");
    local_src.write_str("bare colon content").unwrap();

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), ":"])
        .assert()
        .success();

    let landed = std::env::current_dir().unwrap().join("bare_colon.txt");
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        "bare colon content"
    );
    std::fs::remove_file(&landed).unwrap();
}
