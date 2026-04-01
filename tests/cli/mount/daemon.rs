//! Integration tests for running mount in daemon (non-foreground) mode.

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// DMN-01: Spawning a mount without `--foreground` should launch a daemon
/// that serves filesystem content until killed.
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
    let mount_point = mount_dir.path().to_path_buf();
    std::fs::create_dir_all(&mount_point).expect("failed to create mount point");

    let mut cmd = ctx.new_std_cmd(["mount"]);
    cmd.arg("--backend")
        .arg(mount_backend.as_str())
        .arg("--remote-root")
        .arg(&dir)
        .arg(&mount_point);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn daemon mount process");

    let stdout = child.stdout.take().expect("stdout was not piped");
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if line.contains("Mounted") {
                let _ = tx.send(());
                return;
            }
        }
    });

    rx.recv_timeout(Duration::from_secs(30))
        .expect("daemon mount did not print 'Mounted' within 30s");

    let canonical = std::fs::canonicalize(&mount_point)
        .unwrap_or_else(|e| panic!("failed to canonicalize mount point: {e}"));

    let entries: Vec<_> = std::fs::read_dir(&canonical)
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

    // Unmount first (before killing), then kill the process tree.
    let mp_str = canonical.to_string_lossy();

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("diskutil")
            .args(["unmount", "force", &mp_str])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(unix)]
    {
        let _ = std::process::Command::new("umount")
            .arg("-f")
            .arg(&*mp_str)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    distant_test_harness::process::kill_process_tree(&mut child);

    mount::wait_for_unmount(&canonical);
    let _ = std::fs::remove_dir_all(&canonical);
}
