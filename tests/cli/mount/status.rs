//! Integration tests for the `distant mount-status` CLI subcommand.
//!
//! Verifies that active mounts appear in status output, that JSON format
//! produces valid output, and that no mounts yields an appropriate message.
//!
//! Note: `mount-status` does not accept `--unix-socket` / `--windows-pipe`
//! because it does not connect to the manager.

use assert_cmd::Command;
use rstest::*;

use distant_test_harness::manager::*;

use super::*;

fn mount_status_cmd() -> Command {
    let mut cmd = Command::new(bin_path());
    cmd.arg("mount-status");
    cmd
}

/// MST-01: An active mount should appear in `distant mount-status` output.
#[rstest]
#[test_log::test]
fn mount_status_should_show_active_mount(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let output = mount_status_cmd()
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8_lossy(&output);
        let mount_str = mount.mount_point().to_string_lossy();
        assert!(
            stdout.contains(mount_str.as_ref()),
            "[{backend}] mount-status should contain '{mount_str}', got: {stdout}"
        );
    }
}

/// MST-02: `distant mount-status --format json` should produce valid JSON.
#[rstest]
#[test_log::test]
fn mount_status_json_should_be_valid(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let _mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let output = mount_status_cmd()
            .args(["--format", "json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let stdout = String::from_utf8_lossy(&output);
        let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!("[{backend}] mount-status JSON parse failed: {e}\nraw: {stdout}")
        });

        assert!(
            parsed.is_array(),
            "[{backend}] mount-status JSON should be an array, got: {parsed}"
        );
    }
}

/// MST-03: With no active mounts, `distant mount-status` should report
/// "No mounts found".
///
/// Aggressively cleans up any stale localhost NFS mounts left by prior
/// tests and polls until the mount table is clear.
#[rstest]
#[test_log::test]
fn mount_status_should_show_none_when_empty(_ctx: ManagerCtx) {
    cleanup_all_stale_mounts();

    let output = mount_status_cmd()
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8_lossy(&output);

    assert!(
        stdout.contains("No mounts found"),
        "mount-status with no mounts should say 'No mounts found', got: {stdout}"
    );
}

/// Force-unmount all stale localhost NFS mounts and poll until the mount
/// table is clear.
#[cfg(unix)]
fn cleanup_all_stale_mounts() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    loop {
        let output = match std::process::Command::new("mount")
            .stdout(std::process::Stdio::piped())
            .output()
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return,
        };

        let stale: Vec<String> = output
            .lines()
            .filter(|line| {
                let is_nfs = line.contains("localhost:/") && line.contains("nfs");
                let is_fuse = line.starts_with("distant ") || line.contains("FSName=distant");
                is_nfs || is_fuse
            })
            .filter_map(|line| {
                line.split(" on ")
                    .nth(1)
                    .and_then(|rest| rest.split(" (").next())
                    .map(|s| s.to_string())
            })
            .collect();

        if stale.is_empty() {
            return;
        }

        for path in &stale {
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(path)
                .output();
        }

        if std::time::Instant::now() >= deadline {
            eprintln!("warning: stale mounts still present after 10s: {stale:?}");
            return;
        }

        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(not(unix))]
fn cleanup_all_stale_mounts() {}
