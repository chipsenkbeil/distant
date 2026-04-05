//! Integration tests for reading file contents through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// FRD-01: Reading a seeded text file through the mount should return the
/// exact content written via `cli_write`.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn read_should_return_file_contents(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-read");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");

    let content = std::fs::read_to_string(sm.mount_point.join(&subdir_name).join("hello.txt"))
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

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-read-large");

    let large_content = "A".repeat(100 * 1024);
    ctx.cli_write(&ctx.child_path(&subdir, "large.txt"), &large_content);

    let content = std::fs::read_to_string(sm.mount_point.join(&subdir_name).join("large.txt"))
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

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (_subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-read-noent");

    let result = std::fs::read_to_string(sm.mount_point.join(&subdir_name).join("nonexistent.txt"));

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] reading nonexistent file should fail"
    );
}
