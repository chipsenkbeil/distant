//! Integration tests for directory operations through a mounted directory.

use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// DOP-01: Creating a directory through the mount should propagate to the
/// remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mkdir_should_appear_on_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-dir-create");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    std::fs::create_dir(mp.mount_point().join("new-dir"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to create new-dir: {e}"));

    std::thread::sleep(Duration::from_millis(500));

    assert!(
        ctx.cli_exists(&ctx.child_path(&dir, "new-dir")),
        "[{backend:?}/{mount}] new-dir should exist on remote"
    );
}

/// DOP-02: Removing an empty directory through the mount should delete it
/// from the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rmdir_should_remove_from_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-dir-remove");
    ctx.cli_mkdir(&dir);
    ctx.cli_mkdir(&ctx.child_path(&dir, "empty-dir"));

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    std::fs::remove_dir(mp.mount_point().join("empty-dir"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to remove empty-dir: {e}"));

    std::thread::sleep(Duration::from_millis(500));

    assert!(
        !ctx.cli_exists(&ctx.child_path(&dir, "empty-dir")),
        "[{backend:?}/{mount}] empty-dir should be removed from remote"
    );
}

/// DOP-03: Listing an empty directory through the mount should return zero
/// entries.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn empty_dir_should_list_nothing(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-dir-empty");
    ctx.cli_mkdir(&dir);
    ctx.cli_mkdir(&ctx.child_path(&dir, "empty-dir"));

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let entries: Vec<_> = std::fs::read_dir(mp.mount_point().join("empty-dir"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read empty-dir: {e}"))
        .filter_map(|entry| entry.ok())
        .collect();

    assert!(
        entries.is_empty(),
        "[{backend:?}/{mount}] empty-dir should have no entries, got: {entries:?}"
    );
}
