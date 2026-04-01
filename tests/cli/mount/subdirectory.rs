//! Integration tests for navigating subdirectories through a mounted directory.

use std::collections::HashSet;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// SDT-01: Listing a subdirectory through the mount should show the seeded
/// file and nested directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn subdir_should_list_contents(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-subdir");
    ctx.cli_mkdir(&dir);

    let subdir = ctx.child_path(&dir, "subdir");
    ctx.cli_mkdir(&subdir);
    ctx.cli_write(&ctx.child_path(&subdir, "nested.txt"), "nested content");

    let deep = ctx.child_path(&subdir, "deep");
    ctx.cli_mkdir(&deep);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let entries: HashSet<String> = std::fs::read_dir(mp.mount_point().join("subdir"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read subdir: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.contains("nested.txt"),
        "[{backend:?}/{mount}] expected nested.txt in subdir, got: {entries:?}"
    );
    assert!(
        entries.contains("deep"),
        "[{backend:?}/{mount}] expected deep/ in subdir, got: {entries:?}"
    );
}

/// SDT-02: Reading a deeply nested file through the mount should return the
/// exact content written via `cli_write`.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn deeply_nested_file_should_be_readable(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-deep");
    ctx.cli_mkdir(&dir);

    let subdir = ctx.child_path(&dir, "subdir");
    ctx.cli_mkdir(&subdir);

    let deep = ctx.child_path(&subdir, "deep");
    ctx.cli_mkdir(&deep);
    ctx.cli_write(&ctx.child_path(&deep, "deeper.txt"), "deep content");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let content = std::fs::read_to_string(
        mp.mount_point()
            .join("subdir")
            .join("deep")
            .join("deeper.txt"),
    )
    .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read deeper.txt: {e}"));

    assert_eq!(
        content, "deep content",
        "[{backend:?}/{mount}] deeply nested file content mismatch"
    );
}
