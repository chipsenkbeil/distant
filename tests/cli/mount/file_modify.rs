//! Integration tests for modifying existing file contents through a mounted directory.

use std::io::Write;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// FMD-01: Overwriting an existing file through the mount should sync the
/// new content to the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn overwrite_file_should_sync_to_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) =
        mount::unique_subdir(&ctx, &sm.remote_root, "mount-modify-overwrite");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");

    std::fs::write(
        sm.mount_point.join(&subdir_name).join("hello.txt"),
        "overwritten",
    )
    .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to overwrite hello.txt: {e}"));

    let remote_path = ctx.child_path(&subdir, "hello.txt");
    mount::wait_until_content(&ctx, &remote_path, "overwritten");

    assert_eq!(
        ctx.cli_read(&remote_path),
        "overwritten",
        "[{backend:?}/{mount}] remote content should be overwritten"
    );
}

/// FMD-02: Appending to an existing file through the mount should sync the
/// combined content to the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn append_to_file_should_sync_to_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-modify-append");
    ctx.cli_write(&ctx.child_path(&subdir, "hello.txt"), "hello world");

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(sm.mount_point.join(&subdir_name).join("hello.txt"))
        .unwrap_or_else(|e| {
            panic!("[{backend:?}/{mount}] failed to open hello.txt for append: {e}")
        });

    file.write_all(b" appended")
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to append to hello.txt: {e}"));

    drop(file);

    let remote_path = ctx.child_path(&subdir, "hello.txt");
    mount::wait_until_content(&ctx, &remote_path, "hello world appended");

    assert_eq!(
        ctx.cli_read(&remote_path),
        "hello world appended",
        "[{backend:?}/{mount}] remote content should include appended text"
    );
}
