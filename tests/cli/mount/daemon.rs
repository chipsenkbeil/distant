//! Integration tests for daemon-mode (non-foreground) mounts.
//!
//! Verifies that `distant mount` without `--foreground` successfully daemonizes,
//! serves the mounted filesystem, and can be cleaned up by killing the
//! background process.

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// DMN-01: A daemon-mode mount should successfully expose the remote directory
/// for listing.
///
/// Spawns `distant mount` without `--foreground`, waits for the parent to print
/// "Mounted at <path>" and exit, verifies the mount directory has content, then
/// kills the background daemon and cleans up.
#[rstest]
#[test_log::test]
fn daemon_mount_should_list_directory(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();

        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.arg("--backend")
            .arg(backend)
            .arg("--remote-root")
            .arg(seed_dir.path())
            .arg(mount_dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("[{backend}] failed to spawn daemon mount: {e}"));

        let stdout = child.stdout.take().expect("stdout should be piped");
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("Mounted") {
                    let _ = tx.send(Ok(line));
                    return;
                }
            }
            let _ = tx.send(Err(
                "daemon mount closed stdout without printing 'Mounted'".to_string()
            ));
        });

        let mount_line = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(line)) => line,
            Ok(Err(err)) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("[{backend}] daemon mount failed: {err}");
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("[{backend}] timed out waiting for daemon mount");
            }
        };

        // The parent process should exit after printing "Mounted at ...".
        // Wait for it to finish.
        let parent_status = child
            .wait()
            .unwrap_or_else(|e| panic!("[{backend}] failed to wait on daemon parent: {e}"));

        assert!(
            parent_status.success(),
            "[{backend}] daemon parent should exit successfully, got: {parent_status}"
        );

        // Canonicalize for macOS /var -> /private/var
        let canonical_mount = std::fs::canonicalize(mount_dir.path())
            .unwrap_or_else(|_| mount_dir.path().to_path_buf());

        // Verify the mount has content.
        let entries: Vec<String> = std::fs::read_dir(&canonical_mount)
            .unwrap_or_else(|e| panic!("[{backend}] failed to read mount directory: {e}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(
            !entries.is_empty(),
            "[{backend}] daemon mount should have entries, got none.\
             \nMount line: {mount_line}"
        );

        assert!(
            entries.contains(&"hello.txt".to_string()),
            "[{backend}] daemon mount should contain hello.txt, got: {entries:?}"
        );

        // Kill the background daemon process. On Unix, the daemon re-execs
        // with --foreground, so we find and kill that process.
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("pkill")
                .args(["-f", "distant.*mount.*--foreground"])
                .output();
        }

        // Force-unmount the mount point.
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(&canonical_mount)
                .output();
            let _ = std::process::Command::new("diskutil")
                .args(["unmount", "force"])
                .arg(&canonical_mount)
                .output();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(&canonical_mount)
                .output();
        }

        wait_for_unmount(&canonical_mount);
        let _ = std::fs::remove_dir_all(&canonical_mount);
    }
}
