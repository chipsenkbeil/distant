//! Integration tests for the `distant copy` CLI subcommand.
//!
//! Tests local-to-remote and remote-to-local file transfers including single
//! files, directories, error cases, and edge cases like empty files and binary
//! content. This is distinct from `distant fs copy` which copies within the
//! remote filesystem.

use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

const FILE_CONTENTS: &str = "some text\non multiple lines\nthat is a file's contents\n";

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_upload_single_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("local.txt");
    local_src.write_str(FILE_CONTENTS).unwrap();

    let remote_dir = ctx.unique_dir("copy-upload-single");
    ctx.cli_mkdir(&remote_dir);
    let remote_dst = ctx.child_path(&remote_dir, "remote.txt");
    let remote_path = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    let content = ctx.cli_read(&remote_dst);
    assert_eq!(content, FILE_CONTENTS);
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_download_single_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let remote_dir = ctx.unique_dir("copy-download-single");
    ctx.cli_mkdir(&remote_dir);
    let remote_src = ctx.child_path(&remote_dir, "remote.txt");
    ctx.cli_write(&remote_src, FILE_CONTENTS);

    let local_dst = temp.child("local.txt");
    let remote_path = format!(":{remote_src}");

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert(FILE_CONTENTS);
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
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

    let remote_dir = ctx.unique_dir("copy-upload-dir");
    let remote_dst = ctx.child_path(&remote_dir, "dst_dir");
    let remote_path = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args(["-r", local_dir.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    let a_path = ctx.child_path(&remote_dst, "a.txt");
    assert_eq!(ctx.cli_read(&a_path), "content a");

    let sub_path = ctx.child_path(&remote_dst, "sub");
    let b_path = ctx.child_path(&sub_path, "b.txt");
    assert_eq!(ctx.cli_read(&b_path), "content b");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_download_directory_recursively(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let remote_dir = ctx.unique_dir("copy-download-dir");
    ctx.cli_mkdir(&remote_dir);

    let remote_sub = ctx.child_path(&remote_dir, "sub");
    ctx.cli_mkdir(&remote_sub);

    let remote_file_a = ctx.child_path(&remote_dir, "a.txt");
    ctx.cli_write(&remote_file_a, "content a");

    let remote_file_b = ctx.child_path(&remote_sub, "b.txt");
    ctx.cli_write(&remote_file_b, "content b");

    let local_dir = temp.child("local_dir");
    let remote_path = format!(":{remote_dir}");

    ctx.new_assert_cmd(["copy"])
        .args(["-r", &remote_path, local_dir.to_str().unwrap()])
        .assert()
        .success();

    local_dir.child("a.txt").assert("content a");
    local_dir.child("sub").child("b.txt").assert("content b");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
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
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
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
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_error_when_directory_without_recursive_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_dir = temp.child("a_dir");
    local_dir.create_dir_all().unwrap();

    let remote_dir = ctx.unique_dir("copy-no-recursive");
    let remote_dst = ctx.child_path(&remote_dir, "dst");
    let remote_path = format!(":{remote_dst}");

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
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_error_when_source_not_found(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let missing = temp.child("nonexistent.txt");

    let remote_dir = ctx.unique_dir("copy-not-found");
    let remote_dst = ctx.child_path(&remote_dir, "dst.txt");
    let remote_path = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([missing.to_str().unwrap(), &remote_path])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Failed to read"));
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_upload_into_existing_directory(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_file = temp.child("myfile.txt");
    local_file.write_str("hello").unwrap();

    let remote_dir = ctx.unique_dir("copy-into-existing");
    ctx.cli_mkdir(&remote_dir);
    let remote_path = format!(":{remote_dir}");

    ctx.new_assert_cmd(["copy"])
        .args([local_file.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    let uploaded = ctx.child_path(&remote_dir, "myfile.txt");
    assert_eq!(ctx.cli_read(&uploaded), "hello");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_handle_empty_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let local_src = temp.child("empty.txt");
    local_src.write_str("").unwrap();

    let remote_dir = ctx.unique_dir("copy-empty");
    ctx.cli_mkdir(&remote_dir);
    let remote_dst = ctx.child_path(&remote_dir, "remote_empty.txt");
    let remote_path = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    assert_eq!(ctx.cli_read(&remote_dst), "");

    let local_dst = temp.child("downloaded_empty.txt");
    let remote_path2 = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_dst.to_str().unwrap()])
        .assert()
        .success();

    local_dst.assert("");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_preserve_binary_content(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let binary_data: Vec<u8> = (0..=255).collect();
    let local_src = temp.child("binary.bin");
    local_src.write_binary(&binary_data).unwrap();

    let remote_dir = ctx.unique_dir("copy-binary");
    ctx.cli_mkdir(&remote_dir);
    let remote_dst = ctx.child_path(&remote_dir, "remote_binary.bin");
    let remote_path = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    let local_roundtrip = temp.child("roundtrip.bin");
    let remote_path2 = format!(":{remote_dst}");

    ctx.new_assert_cmd(["copy"])
        .args([&remote_path2, local_roundtrip.to_str().unwrap()])
        .assert()
        .success();

    local_roundtrip.assert(predicate::path::eq_file(local_src.path()));
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_overwrite_existing_destination(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    let remote_dir = ctx.unique_dir("copy-overwrite");
    ctx.cli_mkdir(&remote_dir);
    let remote_file = ctx.child_path(&remote_dir, "target.txt");
    ctx.cli_write(&remote_file, "old content");

    let local_src = temp.child("new.txt");
    local_src.write_str("new content").unwrap();

    let remote_path = format!(":{remote_file}");

    ctx.new_assert_cmd(["copy"])
        .args([local_src.to_str().unwrap(), &remote_path])
        .assert()
        .success();

    assert_eq!(ctx.cli_read(&remote_file), "new content");
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
