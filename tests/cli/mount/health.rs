//! Mount health monitoring tests (HLT-*).
//!
//! Cross-cuts the per-mount monitor task, the kill-leak fix, and
//! the generic event subscription bus. These tests use isolated
//! manager+server pairs (not the singleton) so killing connections
//! and tearing down mounts in arbitrary patterns doesn't affect
//! other tests.
//!
//! Unit tests for the state machine and the monitor's pure helper
//! functions live in `distant-core::net::manager::server::tests`.

use std::time::Duration;

use distant_test_harness::manager;
use distant_test_harness::mount::MountBackend;

/// HLT-05: Killing a connection that owns mounts should clean those
/// mounts up. Without the kill-leak fix, the mounts orphan in the
/// manager's `self.mounts` map with stale `Active` status.
#[test_log::test]
fn kill_should_remove_mounts_owned_by_connection() {
    // Skip on platforms without NFS support to keep this test
    // dependency-free.
    #[cfg(not(feature = "mount-nfs"))]
    return;

    #[cfg(feature = "mount-nfs")]
    {
        // Isolated manager+server so other tests aren't affected.
        let isolated = manager::HostManagerCtx::start();

        // Discover the connection_id via `distant status --show connection --format json`.
        // The JSON shape is a `{ "<id>": "<destination>", ... }` map.
        let list_output = isolated
            .new_std_cmd(["status"])
            .args(["--show", "connection", "--format", "json"])
            .output()
            .expect("status --show connection failed");
        assert!(
            list_output.status.success(),
            "status --show connection returned non-zero: {}",
            String::from_utf8_lossy(&list_output.stderr)
        );
        let stdout = String::from_utf8_lossy(&list_output.stdout);
        let connections: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("status JSON should parse");
        let connection_id: u32 = connections
            .as_object()
            .and_then(|m| m.keys().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("could not find connection id in status JSON: {stdout}"));

        // Seed a directory and mount it.
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let remote_root = std::env::temp_dir()
            .join(format!("distant-hlt05-{}", rand::random::<u32>()))
            .to_string_lossy()
            .into_owned();
        std::fs::create_dir_all(&remote_root).expect("failed to create remote root");

        let mount_output = isolated
            .new_std_cmd(["mount"])
            .arg("--backend")
            .arg(MountBackend::Nfs.as_str())
            .arg("--remote-root")
            .arg(&remote_root)
            .arg(mount_dir.path())
            .output()
            .expect("mount failed");
        assert!(
            mount_output.status.success(),
            "mount should succeed: {}",
            String::from_utf8_lossy(&mount_output.stderr)
        );

        // Verify the mount exists in status.
        let status_before = isolated
            .new_std_cmd(["status"])
            .args(["--show", "mount"])
            .output()
            .expect("status before kill failed");
        let stdout = String::from_utf8_lossy(&status_before.stdout);
        assert!(
            stdout.contains("nfs"),
            "status before kill should list the nfs mount, got:\n{stdout}"
        );

        // Kill the connection that owns the mount.
        let kill_output = isolated
            .new_std_cmd(["kill"])
            .arg(connection_id.to_string())
            .output()
            .expect("kill failed");
        assert!(
            kill_output.status.success(),
            "kill should succeed: {}",
            String::from_utf8_lossy(&kill_output.stderr)
        );

        // Poll status until the mount is gone (or timeout). The kill
        // path is async so the mount cleanup may not be visible
        // immediately.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let status_after = isolated
                .new_std_cmd(["status"])
                .args(["--show", "mount"])
                .output()
                .expect("status after kill failed");
            let stdout = String::from_utf8_lossy(&status_after.stdout);
            if !stdout.contains("nfs") {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "mount should be cleaned up after killing its owning connection, \
                     but status still shows it after 10s:\n{stdout}"
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        // Force-cleanup the mount point in case the OS-side mount
        // somehow lingers.
        let _ = std::process::Command::new("diskutil")
            .args(["unmount", "force", mount_dir.path().to_str().unwrap_or("")])
            .output();
        let _ = std::fs::remove_dir_all(&remote_root);
    }
}
