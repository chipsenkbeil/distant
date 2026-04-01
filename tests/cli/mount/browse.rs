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

/// MNT-02: Dropping the mount process should unmount the directory, leaving
/// the mount point empty or removed.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mount_foreground_should_exit_on_kill(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-fg-kill");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "canary.txt"), "alive");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mount_path = mount_dir.path().to_path_buf();

    {
        let mp = MountProcess::spawn(&ctx, mount, &mount_path, &["--remote-root", &dir]);

        let entries: Vec<_> = std::fs::read_dir(mp.mount_point())
            .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount point: {e}"))
            .filter_map(|entry| entry.ok())
            .collect();

        assert!(
            !entries.is_empty(),
            "[{backend:?}/{mount}] mount point should have entries before drop"
        );
    }

    // After drop, the mount point should be empty or removed entirely.
    match std::fs::read_dir(&mount_path) {
        Ok(mut rd) => {
            assert!(
                rd.next().is_none(),
                "[{backend:?}/{mount}] mount point should be empty after drop"
            );
        }
        Err(_) => {
            // Directory was removed — also acceptable.
        }
    }
}

/// MNT-03: Mounting without `--remote-root` should default to the server's
/// current working directory and expose at least one entry.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mount_should_default_to_server_cwd(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &[]);

    let entries: Vec<_> = std::fs::read_dir(mp.mount_point())
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount point: {e}"))
        .filter_map(|entry| entry.ok())
        .collect();

    assert!(
        !entries.is_empty(),
        "[{backend:?}/{mount}] server cwd should contain at least one entry"
    );
}
