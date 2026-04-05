//! Integration tests for navigating subdirectories through a mounted directory.

use std::collections::HashSet;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// SDT-01: Listing a subdirectory through the mount should show the seeded
/// file and nested directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn subdir_should_list_contents(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-subdir");

    let nested = ctx.child_path(&subdir, "subdir");
    ctx.cli_mkdir(&nested);
    ctx.cli_write(&ctx.child_path(&nested, "nested.txt"), "nested content");

    let deep = ctx.child_path(&nested, "deep");
    ctx.cli_mkdir(&deep);

    let entries: HashSet<String> =
        std::fs::read_dir(sm.mount_point.join(&subdir_name).join("subdir"))
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

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-deep");

    let nested = ctx.child_path(&subdir, "subdir");
    ctx.cli_mkdir(&nested);

    let deep = ctx.child_path(&nested, "deep");
    ctx.cli_mkdir(&deep);
    ctx.cli_write(&ctx.child_path(&deep, "deeper.txt"), "deep content");

    let content = std::fs::read_to_string(
        sm.mount_point
            .join(&subdir_name)
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
