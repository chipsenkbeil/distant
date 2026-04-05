//! Integration tests for `distant unmount`.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::manager;
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

    std::mem::forget(mp);
}

/// UMT-02: `unmount --all` should remove multiple mounts.
///
/// Uses an isolated manager (not the singleton) so `--all` doesn't
/// destroy mounts used by other tests.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_all_should_remove_everything(
    #[case] backend: Backend,
    #[case] mount_backend: MountBackend,
) {
    // Skip non-host backends — the isolated manager only has a host server.
    // SSH/Docker would need their own server setup.
    if !matches!(backend, Backend::Host) {
        return;
    }

    let ctx = skip_if_no_backend!(backend);

    // FP needs its own infrastructure; skip for this isolated-manager test.
    if matches!(mount_backend, MountBackend::MacosFileProvider) {
        return;
    }

    // Start an isolated manager+server so --all only affects our mounts.
    let isolated = manager::HostManagerCtx::start();

    let dir1 = ctx.unique_dir("mount-unmount-all-1");
    ctx.cli_mkdir(&dir1);
    ctx.cli_write(&ctx.child_path(&dir1, "a.txt"), "a");

    let dir2 = ctx.unique_dir("mount-unmount-all-2");
    ctx.cli_mkdir(&dir2);
    ctx.cli_write(&ctx.child_path(&dir2, "b.txt"), "b");

    let mp1_dir = assert_fs::TempDir::new().unwrap();
    let mp2_dir = assert_fs::TempDir::new().unwrap();

    // Mount two filesystems through the isolated manager.
    let mount1 = mount_via_cmd(&isolated, mount_backend, mp1_dir.path(), &dir1);
    let mount2 = mount_via_cmd(&isolated, mount_backend, mp2_dir.path(), &dir2);

    // Verify both are mounted
    assert!(
        mount1.is_some() && mount2.is_some(),
        "[{backend:?}/{mount_backend}] both mounts should succeed"
    );

    // Unmount --all through the isolated manager
    let output = isolated
        .new_std_cmd(["unmount"])
        .arg("--all")
        .output()
        .expect("failed to run unmount --all");

    assert!(
        output.status.success(),
        "[{backend:?}/{mount_backend}] unmount --all should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify both mount points are empty/removed
    mount::wait_for_unmount(mp1_dir.path());
    mount::wait_for_unmount(mp2_dir.path());
}

/// Mounts via the given manager context's CLI. Returns the mount ID on success.
fn mount_via_cmd(
    ctx: &manager::HostManagerCtx,
    mount_backend: MountBackend,
    mount_point: &std::path::Path,
    remote_root: &str,
) -> Option<u32> {
    std::fs::create_dir_all(mount_point).ok()?;
    let output = ctx
        .new_std_cmd(["mount"])
        .arg("--backend")
        .arg(mount_backend.as_str())
        .arg("--remote-root")
        .arg(remote_root)
        .arg(mount_point)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|l| l.contains("Mounted"))
        .and_then(|l| l.rsplit("id: ").next())
        .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok())
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
