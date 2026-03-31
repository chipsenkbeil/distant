//! Integration tests for directory operations through a mounted filesystem.
//!
//! Verifies that creating and removing directories via standard filesystem
//! operations on the mount point updates the remote server, and that listing
//! an empty directory returns no entries.

use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// DOP-01: Creating a new directory through the mount should make it appear
/// on the remote.
#[rstest]
#[test_log::test]
fn mkdir_should_appear_on_remote(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        std::fs::create_dir(mount.mount_point().join("new-dir"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to create new-dir: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_exists(&ctx, &seed_dir.path().join("new-dir"));
    }
}

/// DOP-02: Removing an empty directory through the mount should remove it
/// from the remote.
#[rstest]
#[test_log::test]
fn rmdir_should_remove_from_remote(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        std::fs::remove_dir(mount.mount_point().join("empty-dir"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to remove empty-dir: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_not_exists(&ctx, &seed_dir.path().join("empty-dir"));
    }
}

/// DOP-03: Listing an empty directory through the mount should return zero entries.
#[rstest]
#[test_log::test]
fn empty_dir_should_list_nothing(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let entries: Vec<_> = std::fs::read_dir(mount.mount_point().join("empty-dir"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read empty-dir: {e}"))
            .filter_map(|entry| entry.ok())
            .collect();

        assert!(
            entries.is_empty(),
            "[{backend}] empty-dir should have 0 entries, got: {entries:?}"
        );
    }
}
