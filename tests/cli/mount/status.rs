//! Integration tests for `distant mount-status`.

use assert_cmd::Command;
use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::manager;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// MST-01: With an active mount, `mount-status` output should contain the
/// mount point path.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn status_should_show_active_mount(#[case] backend: Backend, #[case] mount: MountBackend) {
    // FP domains aren't visible via mount-status from non-bundle binaries yet
    if matches!(mount, MountBackend::MacosFileProvider) {
        eprintln!("Skipping mount-status for FileProvider (requires app bundle context)");
        return;
    }
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-status-active");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let mount_path_str = mp.mount_point().to_string_lossy().to_string();

    let output = Command::new(manager::bin_path())
        .arg("mount-status")
        .output()
        .expect("failed to run mount-status");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains(&mount_path_str),
        "[{backend:?}/{mount}] mount-status should include mount point '{mount_path_str}', got:\n{stdout}"
    );
}

/// MST-02: `mount-status --format json` should produce valid JSON output
/// containing mount information.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn status_json_should_be_valid(#[case] backend: Backend, #[case] mount: MountBackend) {
    if matches!(mount, MountBackend::MacosFileProvider) {
        eprintln!("Skipping mount-status JSON for FileProvider (requires app bundle context)");
        return;
    }
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-status-json");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let _mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let output = Command::new(manager::bin_path())
        .args(["mount-status", "--format", "json"])
        .output()
        .expect("failed to run mount-status --format json");

    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "[{backend:?}/{mount}] mount-status JSON should be valid, got error: {e}\nraw: {stdout}"
        )
    });

    assert!(
        parsed.is_array(),
        "[{backend:?}/{mount}] mount-status JSON should be an array, got: {parsed}"
    );
}

/// MST-03: With no active mounts, `mount-status` should print "No mounts found".
/// This test does not need the plugin_x_mount template — it tests a single
/// global condition. Using a plain #[test] avoids running it N times.
#[test_log::test]
fn status_no_mounts_should_say_none() {
    // Clean up any stale mounts left by prior tests.
    distant_test_harness::mount::cleanup_all_stale_mounts();

    let output = Command::new(manager::bin_path())
        .arg("mount-status")
        .output()
        .expect("failed to run mount-status");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("No mounts found") || stdout.trim().is_empty(),
        "mount-status with no mounts should indicate none, got:\n{stdout}"
    );
}
