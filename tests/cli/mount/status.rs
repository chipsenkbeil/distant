//! Integration tests for `distant status --show mount`.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::manager;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// MST-01: With an active mount, `status --show mount` output should contain
/// the backend name.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn status_should_show_active_mount(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-status-active");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let _mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let output = ctx
        .new_std_cmd(["status"])
        .args(["--show", "mount"])
        .output()
        .expect("failed to run status --show mount");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains(mount.as_str()),
        "[{backend:?}/{mount}] status --show mount should include backend name '{}', got:\n{stdout}",
        mount.as_str()
    );
}

/// MST-02: `status --show mount --format json` should produce valid JSON
/// output containing mount information.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn status_json_should_be_valid(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-status-json");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let _mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let output = ctx
        .new_std_cmd(["status"])
        .args(["--show", "mount", "--format", "json"])
        .output()
        .expect("failed to run status --show mount --format json");

    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "[{backend:?}/{mount}] status --show mount JSON should be valid, got error: {e}\nraw: {stdout}"
        )
    });

    assert!(
        parsed.is_array(),
        "[{backend:?}/{mount}] status --show mount JSON should be an array, got: {parsed}"
    );
}

/// MST-03: With no active mounts, `status --show mount` should print
/// "No mounts found". Using a plain #[test] avoids running it N times.
#[test_log::test]
fn status_no_mounts_should_say_none() {
    distant_test_harness::mount::cleanup_all_stale_mounts();

    let output = std::process::Command::new(manager::bin_path())
        .args(["status", "--show", "mount"])
        .output()
        .expect("failed to run status --show mount");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("No mounts found") || stdout.trim().is_empty(),
        "status --show mount with no mounts should indicate none, got:\n{stdout}"
    );
}
