//! Integration tests for `distant unmount`.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// UMT-01: Unmounting a specific mount by ID should succeed.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_by_id_should_succeed(#[case] backend: Backend, #[case] mount_backend: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-unmount-id");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount_backend,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let mount_id = mp.mount_id().expect("mount should have returned an ID");

    let output = ctx
        .new_std_cmd(["unmount"])
        .arg(mount_id.to_string())
        .output()
        .expect("failed to run unmount");

    assert!(
        output.status.success(),
        "[{backend:?}/{mount_backend}] unmount by ID should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    if !matches!(mount_backend, MountBackend::MacosFileProvider) {
        mount::wait_for_unmount(mp.mount_point());
    }

    // Forget the mount so MountProcess::drop doesn't try to unmount again.
    std::mem::forget(mp);
}

/// UMT-02: `unmount --all` should remove all active mounts.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_all_should_remove_everything(
    #[case] backend: Backend,
    #[case] mount_backend: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-unmount-all");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        mount_backend,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let output = ctx
        .new_std_cmd(["unmount"])
        .arg("--all")
        .output()
        .expect("failed to run unmount --all");

    assert!(
        output.status.success(),
        "[{backend:?}/{mount_backend}] unmount --all should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    if !matches!(mount_backend, MountBackend::MacosFileProvider) {
        mount::wait_for_unmount(mp.mount_point());
    }

    // Forget the mount so MountProcess::drop doesn't try to unmount again.
    std::mem::forget(mp);
}

/// UMT-03: Unmounting a nonexistent ID should not claim success.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_nonexistent_should_fail(#[case] backend: Backend, #[case] mount_backend: MountBackend) {
    let _ = mount_backend;
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["unmount"])
        .arg("99999999")
        .output()
        .expect("failed to run unmount");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !combined.contains("Unmounted 99999999"),
        "unmounting a nonexistent ID should not claim success"
    );
}
