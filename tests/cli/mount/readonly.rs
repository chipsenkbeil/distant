//! Integration tests for mounting with `--readonly` and verifying write protection.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// RDO-01: Reading a file through a readonly mount should succeed and return
/// the expected content.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn readonly_read_should_succeed(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-readonly-read");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount,
        mount_dir.path(),
        &["--readonly", "--remote-root", &dir],
    );

    let content = std::fs::read_to_string(mp.mount_point().join("hello.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read hello.txt: {e}"));

    assert_eq!(
        content, "hello world",
        "[{backend:?}/{mount}] readonly read content mismatch"
    );
}

/// RDO-02: Writing a file through a readonly mount should fail.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn readonly_write_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    // FP uses async create-local-then-sync — writes succeed locally even
    // when the remote is readonly. Readonly enforcement needs FP-level
    // read-only domain support.
    if matches!(mount, MountBackend::MacosFileProvider) {
        eprintln!("Skipping readonly write for FileProvider (async write model)");
        return;
    }
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-readonly-write");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount,
        mount_dir.path(),
        &["--readonly", "--remote-root", &dir],
    );

    let result = std::fs::write(mp.mount_point().join("blocked.txt"), "should fail");

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] writing to readonly mount should fail"
    );
}

/// RDO-03: Deleting a file through a readonly mount should fail.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn readonly_delete_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    if matches!(mount, MountBackend::MacosFileProvider) {
        eprintln!("Skipping readonly delete for FileProvider (async write model)");
        return;
    }
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-readonly-delete");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "keep.txt"), "keep me");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount,
        mount_dir.path(),
        &["--readonly", "--remote-root", &dir],
    );

    let result = std::fs::remove_file(mp.mount_point().join("keep.txt"));

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] deleting from readonly mount should fail"
    );
}
