//! Integration tests for running mount in background mode.
//!
//! Uses `MountProcess::spawn()` to mount via the manager and verify that
//! the mounted filesystem serves content correctly.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// DMN-01: A mount running in the background should serve filesystem content
/// until dropped.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn daemon_mount_should_serve_content(
    #[case] backend: Backend,
    #[case] mount_backend: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-daemon");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "daemon.txt"), "daemon content");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount_backend,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let entries: Vec<String> = std::fs::read_dir(mp.mount_point())
        .unwrap_or_else(|e| {
            panic!("[{backend:?}/{mount_backend}] failed to list daemon mount: {e}")
        })
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.contains(&"daemon.txt".to_string()),
        "[{backend:?}/{mount_backend}] daemon mount should contain daemon.txt, got: {entries:?}"
    );

    let content = std::fs::read_to_string(mp.mount_point().join("daemon.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount_backend}] failed to read daemon.txt: {e}"));

    assert_eq!(
        content, "daemon content",
        "[{backend:?}/{mount_backend}] daemon.txt content mismatch"
    );
}
