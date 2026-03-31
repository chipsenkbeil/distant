//! Integration tests for edge-case mount scenarios.
//!
//! Covers automatic directory creation for mount points, rejection of file
//! paths as mount points, special characters in filenames, rapid read/write
//! data integrity, and graceful cleanup after explicit drop.

use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// EDG-01: Mounting to a path that does not yet exist should auto-create the
/// directory and serve remote content.
#[rstest]
#[test_log::test]
fn mount_should_auto_create_directory(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let parent = assert_fs::TempDir::new().unwrap();
        let nonexistent = parent.path().join("does-not-exist-yet");

        assert!(
            !nonexistent.exists(),
            "[{backend}] precondition: mount point should not exist before mount"
        );

        let mount = MountProcess::spawn(
            &ctx,
            backend,
            &nonexistent,
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        assert!(
            mount.mount_point().exists(),
            "[{backend}] mount point should exist after mount"
        );

        let contents = std::fs::read_to_string(mount.mount_point().join("hello.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read hello.txt: {e}"));

        assert_eq!(
            contents, "hello world",
            "[{backend}] hello.txt contents mismatch after auto-create"
        );
    }
}

/// EDG-02: Mounting to a path that is a regular file (not a directory) should
/// fail with a non-zero exit code.
#[rstest]
#[test_log::test]
fn mount_file_as_mountpoint_should_fail(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let parent = assert_fs::TempDir::new().unwrap();
        let file_path = parent.path().join("not-a-directory");
        std::fs::write(&file_path, "I am a file").unwrap_or_else(|e| {
            panic!("[{backend}] failed to create blocker file: {e}");
        });

        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.arg("--backend")
            .arg(backend)
            .arg("--foreground")
            .arg("--remote-root")
            .arg(seed_dir.path())
            .arg(&file_path);

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("[{backend}] failed to spawn mount: {e}"));

        // The process should exit with a failure within a reasonable time.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    assert!(
                        !status.success(),
                        "[{backend}] mount to a file should fail, but exited with: {status}"
                    );
                    break;
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("[{backend}] mount process did not exit within 15s");
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    panic!("[{backend}] failed to check mount process status: {e}");
                }
            }
        }
    }
}

/// EDG-03: A file with spaces in its name should be readable through the mount.
#[rstest]
#[test_log::test]
fn special_chars_in_filename_should_work(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    let spaced_path = seed_dir.path().join("file with spaces.txt");
    ctx.new_assert_cmd(["fs", "write"])
        .args([spaced_path.to_str().unwrap()])
        .write_stdin("spaced content")
        .assert()
        .success();

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let contents = std::fs::read_to_string(mount.mount_point().join("file with spaces.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read 'file with spaces.txt': {e}"));

        assert_eq!(
            contents, "spaced content",
            "[{backend}] spaced filename contents mismatch"
        );
    }
}

/// EDG-04: Rapidly overwriting a file multiple times should not corrupt data.
/// The final read (both local and remote) must match the last write.
#[rstest]
#[test_log::test]
fn rapid_read_write_should_not_corrupt(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let target = mount.mount_point().join("hello.txt");
        let mut last_content = String::new();

        for i in 0..10 {
            last_content = format!("iteration-{i}");
            std::fs::write(&target, &last_content)
                .unwrap_or_else(|e| panic!("[{backend}] write iteration {i} failed: {e}"));
        }

        // Allow the final write to sync to the remote.
        std::thread::sleep(Duration::from_millis(500));

        let local_read = std::fs::read_to_string(&target)
            .unwrap_or_else(|e| panic!("[{backend}] failed to read after rapid writes: {e}"));

        assert_eq!(
            local_read, last_content,
            "[{backend}] local content should match last write"
        );

        verify_remote_file(&ctx, &seed_dir.path().join("hello.txt"), &last_content);
    }
}

/// EDG-05: Dropping a MountProcess should cleanly unmount and leave no stale
/// entries in the OS mount table.
#[rstest]
#[test_log::test]
fn drop_should_leave_no_stale_mount(ctx: ManagerCtx) {
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

        // Verify the mount is functional before dropping.
        let contents = std::fs::read_to_string(mount.mount_point().join("hello.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read hello.txt before drop: {e}"));
        assert_eq!(
            contents, "hello world",
            "[{backend}] hello.txt contents mismatch before drop"
        );

        let canonical = mount.mount_point().to_path_buf();
        drop(mount);

        // MountProcess::drop calls wait_for_unmount internally. Verify the
        // mount table no longer contains this path.
        let output = std::process::Command::new("mount")
            .stdout(std::process::Stdio::piped())
            .output();

        if let Ok(o) = output {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let mount_str = canonical.to_string_lossy();
            assert!(
                !stdout.contains(mount_str.as_ref()),
                "[{backend}] mount table should not contain '{mount_str}' after drop"
            );
        }
    }
}
