//! Integration tests for the `distant unmount` CLI subcommand.
//!
//! Verifies that individual mounts can be unmounted by path, that `--all`
//! removes all mounts, and that unmounting a nonexistent path fails.
//!
//! Note: `unmount` does not accept `--unix-socket` / `--windows-pipe`
//! because it does not connect to the manager. Tests use raw `Command`
//! instead of `ctx.new_assert_cmd()` for unmount commands.

use std::io::BufRead;
use std::process::Stdio;
use std::time::Duration;

use assert_cmd::Command;
use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// Build an `unmount` command without manager socket args.
fn unmount_cmd() -> Command {
    let mut cmd = Command::new(bin_path());
    cmd.arg("unmount");
    cmd
}

/// Spawn a foreground mount via `new_std_cmd` (not `MountProcess`) and wait
/// for the "Mounted" line on stdout. Returns the child process and the
/// canonical mount point path.
fn spawn_raw_mount(
    ctx: &ManagerCtx,
    backend: &str,
    mount_point: &std::path::Path,
    remote_root: &std::path::Path,
) -> (std::process::Child, std::path::PathBuf) {
    std::fs::create_dir_all(mount_point).unwrap_or_else(|e| {
        panic!(
            "Failed to create mount point {}: {e}",
            mount_point.display()
        )
    });

    let mut cmd = ctx.new_std_cmd(["mount"]);
    cmd.arg("--backend")
        .arg(backend)
        .arg("--foreground")
        .arg("--remote-root")
        .arg(remote_root)
        .arg(mount_point)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("[{backend}] failed to spawn mount process: {e}"));

    let stdout = child.stdout.take().expect("stdout should be piped");
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();

    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if line.contains("Mounted") {
                let _ = tx.send(Ok(line));
                return;
            }
        }
        let _ = tx.send(Err(
            "mount process closed stdout without printing 'Mounted'".to_string(),
        ));
    });

    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("[{backend}] mount process failed: {err}");
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("[{backend}] timed out waiting for mount to become ready");
        }
    }

    let canonical =
        std::fs::canonicalize(mount_point).unwrap_or_else(|_| mount_point.to_path_buf());

    (child, canonical)
}

/// Best-effort cleanup for a raw mount child process.
fn cleanup_raw_mount(child: &mut std::process::Child, mount_point: &std::path::Path) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("umount")
            .arg("-f")
            .arg(mount_point)
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
    wait_for_unmount(mount_point);
    let _ = std::fs::remove_dir_all(mount_point);
}

/// UMT-01: `distant unmount <path>` should successfully unmount an active mount.
#[rstest]
#[test_log::test]
fn unmount_by_path_should_succeed(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let (mut child, canonical) =
            spawn_raw_mount(&ctx, backend, mount_dir.path(), seed_dir.path());

        unmount_cmd()
            .arg(canonical.to_str().unwrap())
            .assert()
            .success();

        std::thread::sleep(Duration::from_millis(500));

        let still_has_content = std::fs::read_dir(&canonical)
            .map(|entries| entries.filter_map(|e| e.ok()).count() > 0)
            .unwrap_or(false);

        assert!(
            !still_has_content,
            "[{backend}] mount point should be empty after unmount"
        );

        cleanup_raw_mount(&mut child, &canonical);
    }
}

/// UMT-02: `distant unmount --all` should remove all active mounts.
#[rstest]
#[test_log::test]
fn unmount_all_should_remove_everything(ctx: ManagerCtx) {
    let seed_a = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_a.path());

    let seed_b = assert_fs::TempDir::new().unwrap();
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", seed_b.path().to_str().unwrap()])
        .assert()
        .success();
    ctx.new_assert_cmd(["fs", "write"])
        .args([seed_b.path().join("beta.txt").to_str().unwrap()])
        .write_stdin("beta content")
        .assert()
        .success();

    for backend in available_backends() {
        let mount_a = assert_fs::TempDir::new().unwrap();
        let mount_b = assert_fs::TempDir::new().unwrap();

        let (mut child_a, canonical_a) =
            spawn_raw_mount(&ctx, backend, mount_a.path(), seed_a.path());
        let (mut child_b, canonical_b) =
            spawn_raw_mount(&ctx, backend, mount_b.path(), seed_b.path());

        unmount_cmd().arg("--all").assert().success();

        std::thread::sleep(Duration::from_millis(500));

        let a_has_content = std::fs::read_dir(&canonical_a)
            .map(|entries| entries.filter_map(|e| e.ok()).count() > 0)
            .unwrap_or(false);
        let b_has_content = std::fs::read_dir(&canonical_b)
            .map(|entries| entries.filter_map(|e| e.ok()).count() > 0)
            .unwrap_or(false);

        assert!(
            !a_has_content,
            "[{backend}] mount A should be empty after unmount --all"
        );
        assert!(
            !b_has_content,
            "[{backend}] mount B should be empty after unmount --all"
        );

        cleanup_raw_mount(&mut child_a, &canonical_a);
        cleanup_raw_mount(&mut child_b, &canonical_b);
    }
}

/// UMT-03: Unmounting a path that is not currently mounted should fail.
#[rstest]
#[test_log::test]
fn unmount_nonexistent_should_fail(_ctx: ManagerCtx) {
    unmount_cmd()
        .arg("/some/nonexistent/mount/point/that/does/not/exist")
        .assert()
        .failure();
}
