//! Integration tests for the `distant mount` CLI subcommand.
//!
//! Provides test infrastructure for mounting remote filesystems and verifying
//! file operations through mount points. Tests use `MountProcess` to spawn
//! a foreground mount, then exercise the mounted filesystem via standard I/O
//! and `distant fs` commands.

mod browse;
mod directory_ops;
mod file_create;
mod file_delete;
mod file_modify;
mod file_read;
mod file_rename;
mod multi_mount;
mod readonly;
mod remote_root;
mod status;
mod subdirectory;
mod unmount;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use distant_test_harness::manager::*;

/// Returns the mount backend names available on this platform.
///
/// Each entry corresponds to a `--backend` value accepted by `distant mount`.
/// The `macos-file-provider` backend is excluded because it requires an `.app`
/// bundle and cannot be tested via the CLI directly.
#[allow(dead_code, clippy::vec_init_then_push)]
pub fn available_backends() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut backends = Vec::new();

    #[cfg(feature = "mount-nfs")]
    backends.push("nfs");

    #[cfg(all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ))]
    backends.push("fuse");

    #[cfg(all(feature = "mount-windows-cloud-files", target_os = "windows"))]
    backends.push("windows-cloud-files");

    backends
}

/// A running `distant mount --foreground` process with its mount point.
///
/// On drop, the process is killed, an `unmount` is attempted, and the mount
/// point directory is removed (all best-effort).
#[allow(dead_code)]
pub struct MountProcess {
    child: Child,
    mount_point: PathBuf,
}

#[allow(dead_code)]
impl MountProcess {
    /// Spawn a `distant mount --foreground` process and wait for it to report
    /// "Mounted" on stdout.
    ///
    /// # Panics
    ///
    /// Panics if the process fails to start, exits before printing "Mounted",
    /// or does not print "Mounted" within 30 seconds.
    pub fn spawn(ctx: &ManagerCtx, backend: &str, mount_point: &Path, args: &[&str]) -> Self {
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
            .args(args)
            .arg(mount_point)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mount process: {e}"));

        // Take stdout for the reader thread. stderr stays attached to the
        // child so we can read it on failure.
        let stdout = child.stdout.take().expect("stdout should be piped");

        let (tx, rx) = mpsc::channel::<Result<String, String>>();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("Mounted") {
                    let _ = tx.send(Ok(line));
                    return;
                }
            }

            // stdout closed without "Mounted" — report as error
            let _ = tx.send(Err(
                "mount process closed stdout without printing 'Mounted'".to_string(),
            ));
        });

        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(_line)) => {}
            Ok(Err(err)) => {
                // Read stderr for additional context
                let stderr_msg = Self::read_child_stderr(&mut child);
                let _ = child.kill();
                let _ = child.wait();
                panic!("Mount process failed: {err}\nstderr: {stderr_msg}");
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Timed out waiting for mount process to print 'Mounted'");
            }
        }

        // Canonicalize now while the mount is alive. On macOS, /var is a
        // symlink to /private/var, and the mount table uses the canonical
        // path. If we wait until Drop, canonicalize may fail because the
        // mount process is already dead.
        let canonical_mount =
            std::fs::canonicalize(mount_point).unwrap_or_else(|_| mount_point.to_path_buf());

        Self {
            child,
            mount_point: canonical_mount,
        }
    }

    /// Returns the mount point path.
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    /// Read whatever is available from the child's stderr pipe.
    fn read_child_stderr(child: &mut Child) -> String {
        match child.stderr.take() {
            Some(mut stderr) => {
                let mut buf = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut buf);
                buf
            }
            None => String::new(),
        }
    }
}

impl Drop for MountProcess {
    fn drop(&mut self) {
        // Force-unmount BEFORE killing the mount process. Try both
        // `umount -f` (works for NFS) and `diskutil unmount force`
        // (works for macFUSE). The mount_point was canonicalized at
        // construction time to match the mount table entry.
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(&self.mount_point)
                .output();
            let _ = std::process::Command::new("diskutil")
                .args(["unmount", "force"])
                .arg(&self.mount_point)
                .output();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = std::process::Command::new("umount")
                .arg("-f")
                .arg(&self.mount_point)
                .output();
        }

        // Now kill the foreground mount process.
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        {
            let _ = std::process::Command::new(bin_path())
                .args(["unmount"])
                .arg(&self.mount_point)
                .output();
        }

        // Poll until the mount actually disappears from the OS mount
        // table. Without this, the next test may see a stale mount
        // entry and produce spurious failures.
        wait_for_unmount(&self.mount_point);

        let _ = std::fs::remove_dir_all(&self.mount_point);
    }
}

/// Poll the OS mount table until `mount_point` is no longer listed,
/// or until the timeout expires (5 seconds).
fn wait_for_unmount(mount_point: &Path) {
    let mount_str = mount_point.to_string_lossy();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);

    while std::time::Instant::now() < deadline {
        let output = std::process::Command::new("mount")
            .stdout(std::process::Stdio::piped())
            .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                if !stdout.contains(mount_str.as_ref()) {
                    return;
                }
            }
            Err(_) => return,
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    eprintln!(
        "warning: mount point {} still in mount table after 5s timeout",
        mount_point.display()
    );
}

/// Seed the standard test directory structure on the remote server.
///
/// Creates the following layout under `root`:
/// ```text
/// root/
///   hello.txt          ("hello world")
///   subdir/
///     nested.txt       ("nested content")
///     deep/
///       deeper.txt     ("deep content")
///   empty-dir/
/// ```
#[allow(dead_code)]
pub fn seed_test_data(ctx: &ManagerCtx, root: &Path) {
    let subdir = root.join("subdir");
    let deep = subdir.join("deep");
    let empty_dir = root.join("empty-dir");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", subdir.to_str().unwrap()])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", deep.to_str().unwrap()])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", empty_dir.to_str().unwrap()])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "write"])
        .args([root.join("hello.txt").to_str().unwrap()])
        .write_stdin("hello world")
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "write"])
        .args([subdir.join("nested.txt").to_str().unwrap()])
        .write_stdin("nested content")
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "write"])
        .args([deep.join("deeper.txt").to_str().unwrap()])
        .write_stdin("deep content")
        .assert()
        .success();
}

/// Assert that a remote file has the expected contents.
#[allow(dead_code)]
pub fn verify_remote_file(ctx: &ManagerCtx, path: &Path, expected: &str) {
    ctx.new_assert_cmd(["fs", "read"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout(expected.to_string());
}

/// Assert that a remote path exists.
#[allow(dead_code)]
pub fn verify_remote_exists(ctx: &ManagerCtx, path: &Path) {
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("true\n");
}

/// Assert that a remote path does NOT exist.
#[allow(dead_code)]
pub fn verify_remote_not_exists(ctx: &ManagerCtx, path: &Path) {
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("false\n");
}
