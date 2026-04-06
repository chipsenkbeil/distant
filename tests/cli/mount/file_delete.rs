//! Integration tests for deleting files through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// FDL-01: Removing a file through the mount should delete it from the
/// remote directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn delete_file_should_remove_from_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-delete-file");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name).join("hello.txt"));

    std::fs::remove_file(sm.mount_point.join(&subdir_name).join("hello.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to remove hello.txt: {e}"));

    let remote_path = ctx.child_path(&subdir, "hello.txt");
    mount::wait_until_gone(&ctx, &remote_path);

    assert!(
        !ctx.cli_exists(&remote_path),
        "[{backend:?}/{mount}] hello.txt should be removed from remote"
    );
}

/// FDL-02: Attempting to remove a nonexistent file through the mount should
/// return an error.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn delete_nonexistent_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (_subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-delete-noent");

    let result = std::fs::remove_file(sm.mount_point.join(&subdir_name).join("nonexistent.txt"));

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] removing nonexistent file should fail"
    );
}
