//! Integration tests for mounting remote directories and browsing their contents.

use std::collections::HashSet;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// MNT-01: Mounting with `--remote-root` should expose the seeded directory
/// entries at the mount point root.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mount_should_list_root_directory(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-browse");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");
    ctx.cli_mkdir(&ctx.child_path(&dir, "subdir"));
    ctx.cli_mkdir(&ctx.child_path(&dir, "empty-dir"));

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let entries: HashSet<String> = std::fs::read_dir(mp.mount_point())
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount point: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.contains("hello.txt"),
        "[{backend:?}/{mount}] expected hello.txt, got: {entries:?}"
    );
    assert!(
        entries.contains("subdir"),
        "[{backend:?}/{mount}] expected subdir, got: {entries:?}"
    );
    assert!(
        entries.contains("empty-dir"),
        "[{backend:?}/{mount}] expected empty-dir, got: {entries:?}"
    );
}
