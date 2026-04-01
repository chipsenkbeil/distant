//! Integration tests for modifying existing file contents through a mounted directory.

use std::io::Write;
use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// FMD-01: Overwriting an existing file through the mount should sync the
/// new content to the remote.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn overwrite_file_should_sync_to_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-modify-overwrite");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    std::fs::write(mp.mount_point().join("hello.txt"), "overwritten")
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to overwrite hello.txt: {e}"));

    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(
        ctx.cli_read(&ctx.child_path(&dir, "hello.txt")),
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

    let dir = ctx.unique_dir("mount-modify-append");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(mp.mount_point().join("hello.txt"))
        .unwrap_or_else(|e| {
            panic!("[{backend:?}/{mount}] failed to open hello.txt for append: {e}")
        });

    file.write_all(b" appended")
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to append to hello.txt: {e}"));

    drop(file);

    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(
        ctx.cli_read(&ctx.child_path(&dir, "hello.txt")),
        "hello world appended",
        "[{backend:?}/{mount}] remote content should include appended text"
    );
}
