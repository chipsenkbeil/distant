//! Integration tests for running multiple concurrent mounts.

use std::collections::HashSet;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// MML-01: Two mounts with different remote roots should expose independent
/// content at their respective mount points.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn two_mounts_with_different_roots_should_be_independent(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir_a = ctx.unique_dir("mount-multi-a");
    ctx.cli_mkdir(&dir_a);
    ctx.cli_write(&ctx.child_path(&dir_a, "alpha.txt"), "alpha");

    let dir_b = ctx.unique_dir("mount-multi-b");
    ctx.cli_mkdir(&dir_b);
    ctx.cli_write(&ctx.child_path(&dir_b, "beta.txt"), "beta");

    let mount_a = assert_fs::TempDir::new().unwrap();
    let mount_b = assert_fs::TempDir::new().unwrap();

    let mp_a = MountProcess::spawn(&ctx, mount, mount_a.path(), &["--remote-root", &dir_a]);
    let mp_b = MountProcess::spawn(&ctx, mount, mount_b.path(), &["--remote-root", &dir_b]);

    let entries_a: HashSet<String> = std::fs::read_dir(mp_a.mount_point())
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount A: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    let entries_b: HashSet<String> = std::fs::read_dir(mp_b.mount_point())
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount B: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries_a.contains("alpha.txt"),
        "[{backend:?}/{mount}] mount A should contain alpha.txt, got: {entries_a:?}"
    );
    assert!(
        !entries_a.contains("beta.txt"),
        "[{backend:?}/{mount}] mount A should NOT contain beta.txt, got: {entries_a:?}"
    );
    assert!(
        entries_b.contains("beta.txt"),
        "[{backend:?}/{mount}] mount B should contain beta.txt, got: {entries_b:?}"
    );
    assert!(
        !entries_b.contains("alpha.txt"),
        "[{backend:?}/{mount}] mount B should NOT contain alpha.txt, got: {entries_b:?}"
    );
}

/// MML-02: Dropping one mount should not affect the other mount.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn dropping_one_mount_should_not_affect_other(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir_a = ctx.unique_dir("mount-drop-a");
    ctx.cli_mkdir(&dir_a);
    ctx.cli_write(&ctx.child_path(&dir_a, "alpha.txt"), "alpha");

    let dir_b = ctx.unique_dir("mount-drop-b");
    ctx.cli_mkdir(&dir_b);
    ctx.cli_write(&ctx.child_path(&dir_b, "beta.txt"), "beta");

    let mount_a = assert_fs::TempDir::new().unwrap();
    let mount_b = assert_fs::TempDir::new().unwrap();

    let mp_b = MountProcess::spawn(&ctx, mount, mount_b.path(), &["--remote-root", &dir_b]);

    {
        let _mp_a = MountProcess::spawn(&ctx, mount, mount_a.path(), &["--remote-root", &dir_a]);
    }

    let content =
        std::fs::read_to_string(mp_b.mount_point().join("beta.txt")).unwrap_or_else(|e| {
            panic!("[{backend:?}/{mount}] failed to read beta.txt after drop: {e}")
        });

    assert_eq!(
        content, "beta",
        "[{backend:?}/{mount}] mount B should still work after mount A is dropped"
    );
}

/// MML-03: Mounting the same remote root twice should either succeed (both
/// accessible) or fail gracefully on the second mount.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn same_root_twice_should_work_or_fail_gracefully(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-same-root");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "shared.txt"), "shared");

    let mount_a = assert_fs::TempDir::new().unwrap();
    let mount_b = assert_fs::TempDir::new().unwrap();

    let mp_a = MountProcess::spawn(&ctx, mount, mount_a.path(), &["--remote-root", &dir]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MountProcess::spawn(&ctx, mount, mount_b.path(), &["--remote-root", &dir])
    }));

    let content_a = std::fs::read_to_string(mp_a.mount_point().join("shared.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read from mount A: {e}"));

    assert_eq!(
        content_a, "shared",
        "[{backend:?}/{mount}] first mount should remain accessible"
    );

    match result {
        Ok(mp_b) => {
            let content_b = std::fs::read_to_string(mp_b.mount_point().join("shared.txt"))
                .unwrap_or_else(|e| {
                    panic!("[{backend:?}/{mount}] failed to read from mount B: {e}")
                });
            assert_eq!(
                content_b, "shared",
                "[{backend:?}/{mount}] second mount should also serve the same content"
            );
        }
        Err(_) => {
            log::info!("[{backend:?}/{mount}] second mount with same root failed gracefully");
        }
    }
}
