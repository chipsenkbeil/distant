//! Integration tests for the `distant fs read` CLI subcommand when used on directories.
//!
//! Tests listing directory contents.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_list_directory_entries(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "aaa.txt"), "a");
    ctx.cli_write(&ctx.child_path(&dir, "bbb.txt"), "b");

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs read (directory)");

    assert!(
        output.status.success(),
        "fs read (directory) should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("aaa.txt"),
        "Expected 'aaa.txt' in directory listing, got: {stdout}"
    );
    assert!(
        stdout.contains("bbb.txt"),
        "Expected 'bbb.txt' in directory listing, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_list_subdirectories(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-sub");
    ctx.cli_mkdir(&dir);

    let sub = ctx.child_path(&dir, "subdir");
    ctx.cli_mkdir(&sub);
    ctx.cli_write(&ctx.child_path(&dir, "file.txt"), "content");

    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs read (directory)");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("subdir"),
        "Expected 'subdir' in directory listing, got: {stdout}"
    );
    assert!(
        stdout.contains("file.txt"),
        "Expected 'file.txt' in directory listing, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn yield_an_error_when_fails(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("readdir-err");
    ctx.cli_mkdir(&dir);
    let missing = ctx.child_path(&dir, "missing-dir");

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&missing)
        .assert()
        .code(1);
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_use_absolute_paths_if_specified(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    use distant_test_harness::utils::regex_pred;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file1").touch().unwrap();
    temp.child("file2").touch().unwrap();

    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let expected = {
        let mut s = String::from("^");
        for name in ["file1", "file2"] {
            let escaped = regex::escape(root_path.join(name).to_str().unwrap());
            s.push_str(&format!(r"\s*{escaped}\s*[\r\n]*"));
        }
        s.push('$');
        s
    };

    ctx.new_assert_cmd(["fs", "read"])
        .args(["--absolute", temp.to_str().unwrap()])
        .assert()
        .success()
        .stdout(regex_pred(&expected));
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_canonicalize_flag(#[case] backend: Backend) {
    use assert_fs::prelude::*;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();
    let target_dir = temp.child("target_dir");
    target_dir.create_dir_all().unwrap();
    target_dir.child("file1").touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_dir(target_dir.path()).unwrap();

    let output = ctx
        .new_assert_cmd(["fs", "read"])
        .args(["--canonicalize", "--absolute", link.to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let canonical_target = target_dir.path().canonicalize().unwrap();
    assert!(
        stdout.contains(canonical_target.join("file1").to_str().unwrap()),
        "Expected canonicalized path through symlink to resolve to real target, got: {stdout}"
    );
}
