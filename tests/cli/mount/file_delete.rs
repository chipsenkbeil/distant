//! Integration tests for deleting files through a mounted filesystem.
//!
//! Verifies that removing files via standard filesystem operations on the
//! mount point removes them from the remote server.

use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// FDL-01: Deleting a seeded file through the mount should remove it from the remote.
#[rstest]
#[test_log::test]
fn delete_file_should_remove_from_remote(ctx: ManagerCtx) {
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

        std::fs::remove_file(mount.mount_point().join("hello.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to remove hello.txt: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_not_exists(&ctx, &seed_dir.path().join("hello.txt"));
    }
}

/// FDL-02: Attempting to delete a file that does not exist should return an error.
#[rstest]
#[test_log::test]
fn delete_nonexistent_should_fail(ctx: ManagerCtx) {
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

        let result = std::fs::remove_file(mount.mount_point().join("nonexistent.txt"));

        assert!(
            result.is_err(),
            "[{backend}] removing nonexistent file should fail"
        );
    }
}
