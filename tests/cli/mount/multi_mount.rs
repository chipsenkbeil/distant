//! Integration tests for running multiple simultaneous mounts.
//!
//! Verifies that two mounts can coexist independently, that dropping one does
//! not affect the other, and that mounting the same remote root to two
//! different local paths either succeeds or fails gracefully.

use std::collections::HashSet;
use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// MML-01: Two mounts with different remote roots should expose independent
/// content at their respective mount points.
#[rstest]
#[test_log::test]
fn two_mounts_should_show_independent_content(ctx: ManagerCtx) {
    let seed_a = assert_fs::TempDir::new().unwrap();
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", seed_a.path().to_str().unwrap()])
        .assert()
        .success();
    ctx.new_assert_cmd(["fs", "write"])
        .args([seed_a.path().join("alpha.txt").to_str().unwrap()])
        .write_stdin("alpha content")
        .assert()
        .success();

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

        let proc_a = MountProcess::spawn(
            &ctx,
            backend,
            mount_a.path(),
            &["--remote-root", seed_a.path().to_str().unwrap()],
        );
        let proc_b = MountProcess::spawn(
            &ctx,
            backend,
            mount_b.path(),
            &["--remote-root", seed_b.path().to_str().unwrap()],
        );

        let entries_a: HashSet<String> = std::fs::read_dir(proc_a.mount_point())
            .unwrap_or_else(|e| panic!("[{backend}] failed to read mount A: {e}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        let entries_b: HashSet<String> = std::fs::read_dir(proc_b.mount_point())
            .unwrap_or_else(|e| panic!("[{backend}] failed to read mount B: {e}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(
            entries_a.contains("alpha.txt"),
            "[{backend}] mount A should contain alpha.txt, got: {entries_a:?}"
        );
        assert!(
            !entries_a.contains("beta.txt"),
            "[{backend}] mount A should NOT contain beta.txt, got: {entries_a:?}"
        );

        assert!(
            entries_b.contains("beta.txt"),
            "[{backend}] mount B should contain beta.txt, got: {entries_b:?}"
        );
        assert!(
            !entries_b.contains("alpha.txt"),
            "[{backend}] mount B should NOT contain alpha.txt, got: {entries_b:?}"
        );
    }
}

/// MML-02: Dropping one mount should not affect a second independent mount.
#[rstest]
#[test_log::test]
fn unmount_one_should_not_affect_other(ctx: ManagerCtx) {
    let seed_a = assert_fs::TempDir::new().unwrap();
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", seed_a.path().to_str().unwrap()])
        .assert()
        .success();
    ctx.new_assert_cmd(["fs", "write"])
        .args([seed_a.path().join("alpha.txt").to_str().unwrap()])
        .write_stdin("alpha content")
        .assert()
        .success();

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

        let proc_b = MountProcess::spawn(
            &ctx,
            backend,
            mount_b.path(),
            &["--remote-root", seed_b.path().to_str().unwrap()],
        );

        {
            let _proc_a = MountProcess::spawn(
                &ctx,
                backend,
                mount_a.path(),
                &["--remote-root", seed_a.path().to_str().unwrap()],
            );
        }
        // proc_a dropped here — unmounted and killed

        std::thread::sleep(Duration::from_millis(500));

        let contents = std::fs::read_to_string(proc_b.mount_point().join("beta.txt"))
            .unwrap_or_else(|e| {
                panic!("[{backend}] mount B should still work after dropping mount A: {e}")
            });

        assert_eq!(
            contents, "beta content",
            "[{backend}] mount B content mismatch after dropping mount A"
        );
    }
}

/// MML-03: Mounting the same remote root to two different local paths should
/// either succeed (both serve the same content) or fail gracefully on the
/// second mount.
#[rstest]
#[test_log::test]
fn same_root_twice_should_work_or_error(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_a = assert_fs::TempDir::new().unwrap();
        let mount_b = assert_fs::TempDir::new().unwrap();

        let proc_a = MountProcess::spawn(
            &ctx,
            backend,
            mount_a.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        // The second mount may fail for some backends (e.g., port conflict).
        // We use new_std_cmd to avoid MountProcess::spawn panicking.
        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.arg("--backend")
            .arg(backend)
            .arg("--foreground")
            .arg("--remote-root")
            .arg(seed_dir.path())
            .arg(mount_b.path());

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("[{backend}] failed to spawn second mount: {e}"));

        // Wait for the process to either print "Mounted" or exit.
        let timeout = Duration::from_secs(30);
        let mut second_mounted = false;

        // Read stdout in a thread to detect "Mounted"
        let stdout = child.stdout.take().expect("stdout should be piped");
        let (tx, rx) = std::sync::mpsc::channel::<bool>();

        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
                if line.contains("Mounted") {
                    let _ = tx.send(true);
                    return;
                }
            }
            let _ = tx.send(false);
        });

        match rx.recv_timeout(timeout) {
            Ok(true) => {
                second_mounted = true;
            }
            Ok(false) | Err(_) => {
                // Second mount failed or timed out — that's acceptable
            }
        }

        if second_mounted {
            // Both mounts active — verify both serve the same content
            let contents_a = std::fs::read_to_string(proc_a.mount_point().join("hello.txt"))
                .unwrap_or_else(|e| panic!("[{backend}] failed to read from first mount: {e}"));

            // Canonicalize the second mount point for macOS /var -> /private/var
            let canonical_b = std::fs::canonicalize(mount_b.path())
                .unwrap_or_else(|_| mount_b.path().to_path_buf());
            let contents_b = std::fs::read_to_string(canonical_b.join("hello.txt"))
                .unwrap_or_else(|e| panic!("[{backend}] failed to read from second mount: {e}"));

            assert_eq!(
                contents_a, contents_b,
                "[{backend}] both mounts of same root should serve identical content"
            );
        }

        // Clean up the second mount process
        #[cfg(unix)]
        {
            let canonical_b = std::fs::canonicalize(mount_b.path())
                .unwrap_or_else(|_| mount_b.path().to_path_buf());
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(&canonical_b)
                .output();
        }
        let _ = child.kill();
        let _ = child.wait();

        // proc_a drops here normally
        drop(proc_a);
    }
}
