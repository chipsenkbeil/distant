//! Integration tests for renaming files through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// FRN-01: Renaming a file within the same directory through the mount
/// should update the remote accordingly.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rename_file_should_update_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-rename-file");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name).join("hello.txt"));

    let local_dir = sm.mount_point.join(&subdir_name);
    std::fs::rename(local_dir.join("hello.txt"), local_dir.join("renamed.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to rename hello.txt: {e}"));

    mount::wait_until_gone(&ctx, &ctx.child_path(&subdir, "hello.txt"));
    mount::wait_until_exists(&ctx, &ctx.child_path(&subdir, "renamed.txt"));

    assert!(
        !ctx.cli_exists(&ctx.child_path(&subdir, "hello.txt")),
        "[{backend:?}/{mount}] hello.txt should no longer exist on remote"
    );
    assert!(
        ctx.cli_exists(&ctx.child_path(&subdir, "renamed.txt")),
        "[{backend:?}/{mount}] renamed.txt should exist on remote"
    );
}

/// FRN-02: Renaming a file across directories through the mount should move
/// it to the target subdirectory on the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rename_across_dirs_should_update_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-rename-xdir");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");
    ctx.cli_mkdir(&ctx.child_path(&subdir, "subdir"));

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name).join("hello.txt"));

    let local_dir = sm.mount_point.join(&subdir_name);
    std::fs::rename(
        local_dir.join("hello.txt"),
        local_dir.join("subdir").join("moved.txt"),
    )
    .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed cross-dir rename: {e}"));

    mount::wait_until_gone(&ctx, &ctx.child_path(&subdir, "hello.txt"));

    let moved_path = ctx.child_path(&ctx.child_path(&subdir, "subdir"), "moved.txt");
    mount::wait_until_exists(&ctx, &moved_path);

    assert!(
        !ctx.cli_exists(&ctx.child_path(&subdir, "hello.txt")),
        "[{backend:?}/{mount}] hello.txt should no longer exist on remote"
    );
    assert!(
        ctx.cli_exists(&moved_path),
        "[{backend:?}/{mount}] subdir/moved.txt should exist on remote"
    );
}
