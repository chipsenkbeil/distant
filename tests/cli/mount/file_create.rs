//! Integration tests for creating files through a mounted filesystem.
//!
//! Verifies that new files written via standard filesystem operations on the
//! mount point appear on the remote server with the correct contents.

use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// FCR-01: Writing a new file at the mount root should create it on the remote.
#[rstest]
#[test_log::test]
fn create_file_should_appear_on_remote(ctx: ManagerCtx) {
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

        std::fs::write(mount.mount_point().join("new-file.txt"), "created content")
            .unwrap_or_else(|e| panic!("[{backend}] failed to write new-file.txt: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_file(
            &ctx,
            &seed_dir.path().join("new-file.txt"),
            "created content",
        );
    }
}

/// FCR-02: Writing a new file inside a subdirectory through the mount should
/// create it on the remote in the correct location.
#[rstest]
#[test_log::test]
fn create_file_in_subdir_should_appear_on_remote(ctx: ManagerCtx) {
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

        std::fs::write(
            mount.mount_point().join("subdir").join("new-nested.txt"),
            "nested created",
        )
        .unwrap_or_else(|e| panic!("[{backend}] failed to write new-nested.txt: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_file(
            &ctx,
            &seed_dir.path().join("subdir").join("new-nested.txt"),
            "nested created",
        );
    }
}
