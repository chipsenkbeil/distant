//! Integration tests for the `distant mount` CLI subcommand.
//!
//! Provides test infrastructure for mounting remote filesystems and verifying
//! file operations through mount points. Tests use `MountProcess` to spawn
//! a foreground mount, then exercise the mounted filesystem via standard I/O
//! and `distant fs` commands.

mod browse;
mod daemon;
mod directory_ops;
mod edge_cases;
mod file_create;
mod file_delete;
mod file_modify;
#[cfg(all(target_os = "macos", feature = "mount-macos-file-provider"))]
mod file_provider;
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

/// Build a signed macOS `.app` bundle for FileProvider integration tests.
///
/// Creates a `test-Distant.app` bundle inside the workspace's `target/` directory
/// using the compiled test binary. The appex's `Info.plist` is patched to use
/// the test App Group ID (`group.dev.distant.test`) instead of the production one.
///
/// The bundle is signed ad-hoc and registered with PlugInKit so that macOS
/// recognizes the FileProvider extension.
///
/// Skips the build if the existing bundle binary is newer than the source binary.
///
/// # Panics
///
/// Panics if any filesystem operation, code signing, or PlugInKit registration fails.
#[cfg(all(target_os = "macos", feature = "mount-macos-file-provider"))]
pub fn build_test_app_bundle() -> std::path::PathBuf {
    use std::fs;
    use std::process::Command;

    let source_binary = bin_path();

    // The workspace root is the parent of the binary crate directory.
    // env!("CARGO_MANIFEST_DIR") points to the binary crate's directory,
    // which IS the workspace root for this project.
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = workspace_root.join("target");

    let app_path = target_dir.join("test-Distant.app");
    let contents = app_path.join("Contents");
    let macos_dir = contents.join("MacOS");
    let plugins_dir = contents.join("PlugIns");
    let appex_path = plugins_dir.join("DistantFileProvider.appex");
    let appex_contents = appex_path.join("Contents");
    let appex_macos = appex_contents.join("MacOS");

    let bundle_binary = macos_dir.join("distant");

    // Check if the bundle is already up-to-date by comparing mtimes
    if bundle_binary.exists() {
        let source_mtime = fs::metadata(&source_binary).and_then(|m| m.modified()).ok();
        let bundle_mtime = fs::metadata(&bundle_binary).and_then(|m| m.modified()).ok();

        if let (Some(src), Some(dst)) = (source_mtime, bundle_mtime)
            && dst >= src
        {
            eprintln!("test-Distant.app is up-to-date, skipping rebuild");
            return app_path;
        }
    }

    eprintln!("Building test-Distant.app bundle...");

    // Create the directory structure
    fs::create_dir_all(&macos_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", macos_dir.display()));
    fs::create_dir_all(&appex_macos)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", appex_macos.display()));

    // Copy the test binary into both locations
    fs::copy(&source_binary, &bundle_binary).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} -> {}: {e}",
            source_binary.display(),
            bundle_binary.display()
        )
    });
    let appex_binary = appex_macos.join("distant");
    fs::copy(&source_binary, &appex_binary).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} -> {}: {e}",
            source_binary.display(),
            appex_binary.display()
        )
    });

    // Copy the app's Info.plist as-is
    let resources_dir = workspace_root.join("resources").join("macos");
    let app_plist_src = resources_dir.join("Info.plist");
    let app_plist_dst = contents.join("Info.plist");
    fs::copy(&app_plist_src, &app_plist_dst).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} -> {}: {e}",
            app_plist_src.display(),
            app_plist_dst.display()
        )
    });

    // Copy the appex's Info.plist with the test App Group ID substitution
    let appex_plist_src = resources_dir.join("Extension-Info.plist");
    let appex_plist_content = fs::read_to_string(&appex_plist_src)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", appex_plist_src.display()));
    let appex_plist_patched =
        appex_plist_content.replace("39C6AGD73Z.group.dev.distant", "group.dev.distant.test");
    let appex_plist_dst = appex_contents.join("Info.plist");
    fs::write(&appex_plist_dst, &appex_plist_patched)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", appex_plist_dst.display()));

    // Write test entitlements to a temp file
    let entitlements_content = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
";
    let entitlements_path = target_dir.join("test-distant-entitlements.plist");
    fs::write(&entitlements_path, entitlements_content).unwrap_or_else(|e| {
        panic!(
            "failed to write entitlements to {}: {e}",
            entitlements_path.display()
        )
    });

    // Sign the appex first (inner-to-outer as required by Apple)
    let status = Command::new("codesign")
        .args(["-s", "-", "-f", "--entitlements"])
        .arg(&entitlements_path)
        .arg(&appex_path)
        .status()
        .expect("failed to run codesign on appex");
    assert!(
        status.success(),
        "codesign failed on appex (exit code: {status})"
    );

    // Then sign the app bundle
    let status = Command::new("codesign")
        .args(["-s", "-", "-f", "--entitlements"])
        .arg(&entitlements_path)
        .arg(&app_path)
        .status()
        .expect("failed to run codesign on app");
    assert!(
        status.success(),
        "codesign failed on app bundle (exit code: {status})"
    );

    // Register the appex with PlugInKit
    let status = Command::new("pluginkit")
        .args(["-a"])
        .arg(&appex_path)
        .status()
        .expect("failed to run pluginkit -a");
    assert!(
        status.success(),
        "pluginkit -a failed (exit code: {status})"
    );

    let status = Command::new("pluginkit")
        .args(["-e", "use", "-i", "dev.distant.file-provider"])
        .status()
        .expect("failed to run pluginkit -e use");
    assert!(
        status.success(),
        "pluginkit -e use failed (exit code: {status})"
    );

    eprintln!("test-Distant.app bundle built and signed successfully");
    app_path
}

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
