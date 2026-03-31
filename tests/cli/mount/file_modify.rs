//! Integration tests for modifying existing files through a mounted filesystem.
//!
//! Verifies that overwriting and appending to files via standard filesystem
//! operations on the mount point syncs the updated contents to the remote server.

use std::io::Write;
use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// FMD-01: Overwriting an existing file through the mount should update the
/// remote contents.
#[rstest]
#[test_log::test]
fn overwrite_file_should_sync_to_remote(ctx: ManagerCtx) {
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

        std::fs::write(mount.mount_point().join("hello.txt"), "overwritten")
            .unwrap_or_else(|e| panic!("[{backend}] failed to overwrite hello.txt: {e}"));

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_file(&ctx, &seed_dir.path().join("hello.txt"), "overwritten");
    }
}

/// FMD-02: Appending to an existing file through the mount should add content
/// to the remote file without replacing the original contents.
#[rstest]
#[test_log::test]
fn append_to_file_should_sync_to_remote(ctx: ManagerCtx) {
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

        let hello_path = mount.mount_point().join("hello.txt");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&hello_path)
            .unwrap_or_else(|e| panic!("[{backend}] failed to open hello.txt for append: {e}"));

        file.write_all(b" appended")
            .unwrap_or_else(|e| panic!("[{backend}] failed to append to hello.txt: {e}"));

        drop(file);

        std::thread::sleep(Duration::from_millis(500));

        verify_remote_file(
            &ctx,
            &seed_dir.path().join("hello.txt"),
            "hello world appended",
        );
    }
}
