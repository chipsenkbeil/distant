//! Integration tests for deleting files through a mounted directory.

use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// FDL-01: Removing a file through the mount should delete it from the
/// remote directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn delete_file_should_remove_from_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-delete-file");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    std::fs::remove_file(mp.mount_point().join("hello.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to remove hello.txt: {e}"));

    std::thread::sleep(Duration::from_millis(500));

    assert!(
        !ctx.cli_exists(&ctx.child_path(&dir, "hello.txt")),
        "[{backend:?}/{mount}] hello.txt should be removed from remote"
    );
}

/// FDL-02: Attempting to remove a nonexistent file through the mount should
/// return an error.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn delete_nonexistent_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-delete-noent");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let result = std::fs::remove_file(mp.mount_point().join("nonexistent.txt"));

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] removing nonexistent file should fail"
    );
}
