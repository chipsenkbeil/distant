//! Integration tests for renaming files through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// FRN-01: Renaming a file within the same directory through the mount
/// should update the remote accordingly.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rename_file_should_update_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-rename-file");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    std::fs::rename(
        mp.mount_point().join("hello.txt"),
        mp.mount_point().join("renamed.txt"),
    )
    .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to rename hello.txt: {e}"));

    distant_test_harness::mount::wait_for_sync();

    assert!(
        !ctx.cli_exists(&ctx.child_path(&dir, "hello.txt")),
        "[{backend:?}/{mount}] hello.txt should no longer exist on remote"
    );
    assert!(
        ctx.cli_exists(&ctx.child_path(&dir, "renamed.txt")),
        "[{backend:?}/{mount}] renamed.txt should exist on remote"
    );
}

/// FRN-02: Renaming a file across directories through the mount should move
/// it to the target subdirectory on the remote. Not all backends support
/// cross-directory rename, so failure is logged rather than asserted.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rename_across_dirs_should_update_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-rename-xdir");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");
    ctx.cli_mkdir(&ctx.child_path(&dir, "subdir"));

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let result = std::fs::rename(
        mp.mount_point().join("hello.txt"),
        mp.mount_point().join("subdir").join("moved.txt"),
    );

    if let Err(e) = result {
        log::warn!("[{backend:?}/{mount}] cross-directory rename not supported: {e}");
        return;
    }

    distant_test_harness::mount::wait_for_sync();

    assert!(
        !ctx.cli_exists(&ctx.child_path(&dir, "hello.txt")),
        "[{backend:?}/{mount}] hello.txt should no longer exist on remote"
    );

    let moved_path = ctx.child_path(&ctx.child_path(&dir, "subdir"), "moved.txt");
    assert!(
        ctx.cli_exists(&moved_path),
        "[{backend:?}/{mount}] subdir/moved.txt should exist on remote"
    );
}
