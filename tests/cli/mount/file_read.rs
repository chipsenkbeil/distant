//! Integration tests for reading file contents through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// FRD-01: Reading a seeded text file through the mount should return the
/// exact content written via `cli_write`.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn read_should_return_file_contents(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-read");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let content = std::fs::read_to_string(mp.mount_point().join("hello.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read hello.txt: {e}"));

    assert_eq!(
        content, "hello world",
        "[{backend:?}/{mount}] file content mismatch"
    );
}

/// FRD-02: Reading a 100KB file through the mount should return the full
/// content with correct size.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn read_should_handle_large_file(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-read-large");
    ctx.cli_mkdir(&dir);

    let large_content = "A".repeat(100 * 1024);
    ctx.cli_write(&ctx.child_path(&dir, "large.txt"), &large_content);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let content = std::fs::read_to_string(mp.mount_point().join("large.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read large.txt: {e}"));

    assert_eq!(
        content.len(),
        100 * 1024,
        "[{backend:?}/{mount}] expected 100KB file, got {} bytes",
        content.len()
    );
    assert_eq!(
        content, large_content,
        "[{backend:?}/{mount}] large file content mismatch"
    );
}

/// FRD-03: Attempting to read a nonexistent file through the mount should
/// return an error.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn read_should_fail_for_nonexistent_file(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-read-noent");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let result = std::fs::read_to_string(mp.mount_point().join("nonexistent.txt"));

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] reading nonexistent file should fail"
    );
}
