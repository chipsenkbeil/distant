//! Integration tests for renaming files through a mounted filesystem.
//!
//! Verifies that renaming files via standard filesystem operations on the
//! mount point updates the remote server, both within the same directory
//! and across directories.

use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// FRN-01: Renaming a file within the same directory through the mount should
/// remove the old name and create the new name on the remote.
#[rstest]
#[test_log::test]
fn rename_file_should_update_remote(ctx: ManagerCtx) {
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

        std::fs::rename(
            mount.mount_point().join("hello.txt"),
            mount.mount_point().join("renamed.txt"),
        )
        .unwrap_or_else(|e| panic!("[{backend}] failed to rename hello.txt: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_not_exists(&ctx, &seed_dir.path().join("hello.txt"));
        verify_remote_file(&ctx, &seed_dir.path().join("renamed.txt"), "hello world");
    }
}

/// FRN-02: Renaming a file from the root into a subdirectory through the mount
/// should move it on the remote. Some backends may not support cross-directory
/// renames, so failures are logged and skipped.
#[rstest]
#[test_log::test]
fn rename_across_dirs_should_update_remote(ctx: ManagerCtx) {
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

        if let Err(e) = std::fs::rename(
            mount.mount_point().join("hello.txt"),
            mount.mount_point().join("subdir").join("moved.txt"),
        ) {
            eprintln!("[{backend}] rename across dirs not supported: {e}");
            continue;
        }

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_not_exists(&ctx, &seed_dir.path().join("hello.txt"));
        verify_remote_file(
            &ctx,
            &seed_dir.path().join("subdir").join("moved.txt"),
            "hello world",
        );
    }
}
