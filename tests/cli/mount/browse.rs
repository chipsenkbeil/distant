//! Integration tests for mounting remote directories and browsing their contents.
//!
//! Verifies that a mounted filesystem exposes the expected directory entries,
//! that foreground mounts clean up on kill, and that mounting without
//! `--remote-root` defaults to the server's working directory.

use std::collections::HashSet;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// MNT-01: Mounting with `--remote-root` should expose the seeded directory
/// entries at the mount point root.
#[rstest]
#[test_log::test]
fn mount_should_list_root_directory(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let entries: HashSet<String> = std::fs::read_dir(mount.mount_point())
            .unwrap_or_else(|e| panic!("[{backend}] failed to read mount point: {e}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(
            entries.contains("hello.txt"),
            "[{backend}] expected hello.txt in root, got: {entries:?}"
        );
        assert!(
            entries.contains("subdir"),
            "[{backend}] expected subdir in root, got: {entries:?}"
        );
        assert!(
            entries.contains("empty-dir"),
            "[{backend}] expected empty-dir in root, got: {entries:?}"
        );
    }
}

/// MNT-02: After killing a foreground mount process, the mount point should
/// become an ordinary empty directory again.
#[rstest]
#[test_log::test]
fn mount_foreground_should_exit_on_kill(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount_path = mount_dir.path().to_path_buf();

        {
            let _mount = MountProcess::spawn(
                &ctx,
                backend,
                &mount_path,
                &["--remote-root", seed_dir.path().to_str().unwrap()],
            );

            // Confirm mount is active before dropping
            assert!(
                std::fs::read_dir(&mount_path).is_ok(),
                "[{backend}] mount point should be readable while mounted"
            );
        }
        // MountProcess dropped here -- process killed, unmount attempted

        // Give the OS a moment to finalize the unmount
        std::thread::sleep(std::time::Duration::from_millis(500));

        // The mount point directory may have been removed by MountProcess::drop,
        // or it may still exist but be empty (no longer mounted).
        if mount_path.exists() {
            let entries: Vec<_> = std::fs::read_dir(&mount_path)
                .unwrap_or_else(|e| {
                    panic!("[{backend}] failed to read mount point after kill: {e}")
                })
                .filter_map(|e| e.ok())
                .collect();

            assert!(
                entries.is_empty(),
                "[{backend}] mount point should be empty after kill, got: {entries:?}"
            );
        }
    }
}

/// MNT-03: Mounting without `--remote-root` should default to the server's
/// working directory and succeed with at least one directory entry.
#[rstest]
#[test_log::test]
fn mount_should_default_to_server_cwd(ctx: ManagerCtx) {
    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(&ctx, backend, mount_dir.path(), &[]);

        let entry_count = std::fs::read_dir(mount.mount_point())
            .unwrap_or_else(|e| panic!("[{backend}] failed to read mount point: {e}"))
            .filter_map(|entry| entry.ok())
            .count();

        assert!(
            entry_count > 0,
            "[{backend}] mount with default remote root should expose at least one entry"
        );
    }
}
