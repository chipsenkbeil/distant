//! Integration tests for directory operations through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// DOP-01: Creating a directory through the mount should propagate to the
/// remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mkdir_should_appear_on_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-dir-create");

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name));

    std::fs::create_dir(sm.mount_point.join(&subdir_name).join("new-dir"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to create new-dir: {e}"));

    let remote_path = ctx.child_path(&subdir, "new-dir");
    mount::wait_until_exists(&ctx, &remote_path);

    assert!(
        ctx.cli_exists(&remote_path),
        "[{backend:?}/{mount}] new-dir should exist on remote"
    );
}

/// DOP-02: Removing an empty directory through the mount should delete it
/// from the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rmdir_should_remove_from_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-dir-remove");
    ctx.cli_mkdir(&ctx.child_path(&subdir, "empty-dir"));

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name).join("empty-dir"));

    // FileProvider directories may contain hidden metadata (resource forks),
    // so use remove_dir_all. Other backends use remove_dir for a true empty check.
    if matches!(mount, MountBackend::MacosFileProvider) {
        std::fs::remove_dir_all(sm.mount_point.join(&subdir_name).join("empty-dir"))
            .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to remove empty-dir: {e}"));
    } else {
        std::fs::remove_dir(sm.mount_point.join(&subdir_name).join("empty-dir"))
            .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to remove empty-dir: {e}"));
    }

    let remote_path = ctx.child_path(&subdir, "empty-dir");
    mount::wait_until_gone(&ctx, &remote_path);

    assert!(
        !ctx.cli_exists(&remote_path),
        "[{backend:?}/{mount}] empty-dir should be removed from remote"
    );
}

/// DOP-03: Listing an empty directory through the mount should return zero
/// entries.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn empty_dir_should_list_nothing(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (_subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-dir-empty");

    mount::wait_for_path(mount, &sm.mount_point.join(&subdir_name));

    let entries: Vec<_> = std::fs::read_dir(sm.mount_point.join(&subdir_name))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read empty dir: {e}"))
        .filter_map(|entry| entry.ok())
        .collect();

    assert!(
        entries.is_empty(),
        "[{backend:?}/{mount}] empty-dir should have no entries, got: {entries:?}"
    );
}
