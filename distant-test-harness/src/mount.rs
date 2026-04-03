//! Mount test infrastructure for integration tests.
//!
//! Provides [`MountProcess`] for managing foreground mount lifecycles,
//! [`wait_for_unmount`] for polling unmount completion, and
//! [`build_test_app_bundle`] for constructing macOS FileProvider `.app`
//! bundles in tests. Also re-exports [`MountBackend`] for convenience
//! and defines rstest-reuse templates for parameterized mount tests.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[allow(unused_imports)]
use rstest::rstest;
#[allow(unused_imports)]
use rstest_reuse::{self, *};

pub use distant_mount::MountBackend;

#[allow(unused_imports)]
use crate::backend::Backend;
use crate::backend::BackendCtx;

/// Timeout for waiting for the mount process to emit "Mounted" on stdout.
const MOUNT_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for polling `mount` command output until the mount point disappears.
const UNMOUNT_POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between `mount` command polls in [`wait_for_unmount`].
const UNMOUNT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// A managed mount process for integration tests.
///
/// Wraps a child process running `distant mount --foreground` and handles
/// cleanup (unmount + kill + directory removal) on drop.
///
/// For macOS FileProvider mounts, the child process exits after printing
/// "Mounted" (no foreground needed), and the mount point is discovered
/// under `~/Library/CloudStorage/` rather than being specified by the caller.
pub struct MountProcess {
    child: Child,
    mount_point: PathBuf,

    /// When true, this mount was created via the macOS FileProvider backend.
    /// Cleanup uses `distant unmount --all` via the bundled binary instead of
    /// `umount`/`diskutil`.
    is_file_provider: bool,

    /// Path to the `.app` bundle binary used for FileProvider mounts.
    /// Stored so that Drop can invoke `unmount --all` through the same binary.
    #[cfg(target_os = "macos")]
    bundled_bin: Option<PathBuf>,
}

impl MountProcess {
    /// Spawns a `distant mount` process and waits for it to be ready.
    ///
    /// For FUSE/NFS/WCF backends, uses `ctx.new_std_cmd(["mount"])` to get a
    /// properly configured command, then adds `--backend`, `--foreground`, any
    /// extra `args`, and the `mount_point`. Blocks until the process prints
    /// "Mounted" on stdout (up to 30 seconds). The mount point is then
    /// canonicalized to resolve macOS `/var` to `/private/var` symlinks.
    ///
    /// For macOS FileProvider, builds a `.app` bundle via
    /// [`build_test_app_bundle`] and runs the bundled binary with the context's
    /// socket. The process exits after printing "Mounted" and the mount point
    /// is discovered under `~/Library/CloudStorage/`.
    ///
    /// Returns `Err` if the mount fails to start (process exits early or
    /// doesn't print "Mounted" within the timeout). The caller can use this
    /// to test error cases without leaking processes.
    pub fn try_spawn(
        ctx: &BackendCtx,
        mount: MountBackend,
        mount_point: &Path,
        args: &[&str],
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        if matches!(mount, MountBackend::MacosFileProvider) {
            return Self::try_spawn_file_provider(ctx, args);
        }

        Self::try_spawn_foreground(ctx, mount, mount_point, args)
    }

    /// Spawns a foreground mount process for FUSE/NFS/WCF backends.
    fn try_spawn_foreground(
        ctx: &BackendCtx,
        mount: MountBackend,
        mount_point: &Path,
        args: &[&str],
    ) -> Result<Self, String> {
        if let Err(e) = std::fs::create_dir_all(mount_point) {
            return Err(format!(
                "failed to create mount point {}: {e}",
                mount_point.display()
            ));
        }

        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.arg("--backend").arg(mount.as_str()).arg("--foreground");

        for arg in args {
            cmd.arg(arg);
        }

        cmd.arg(mount_point);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().expect("failed to spawn distant mount process");

        let stdout = child.stdout.take().expect("stdout was not piped");
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("Mounted") {
                    let _ = tx.send(Ok(()));
                    return;
                }
            }
            let _ = tx.send(Err(
                "mount process stdout closed without printing 'Mounted'".to_string(),
            ));
        });

        match rx.recv_timeout(MOUNT_READY_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(msg);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "mount process did not print 'Mounted' within {}s",
                    MOUNT_READY_TIMEOUT.as_secs()
                ));
            }
        }

        let canonical = match std::fs::canonicalize(mount_point) {
            Ok(p) => p,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "failed to canonicalize mount point {}: {e}",
                    mount_point.display()
                ));
            }
        };

        Ok(Self {
            child,
            mount_point: canonical,
            is_file_provider: false,
            #[cfg(target_os = "macos")]
            bundled_bin: None,
        })
    }

    /// Spawns a macOS FileProvider mount using the bundled `.app` binary.
    ///
    /// The bundled binary talks to the same manager via the context's unix
    /// socket. The process prints "Mounted" and then exits. The mount point
    /// is discovered by scanning `~/Library/CloudStorage/` for new entries.
    #[cfg(target_os = "macos")]
    fn try_spawn_file_provider(ctx: &BackendCtx, args: &[&str]) -> Result<Self, String> {
        use crate::manager;

        let app_dir = build_test_app_bundle();
        let bundled_binary = app_dir.join("Contents").join("MacOS").join("distant");

        let cloud_storage = dirs_cloud_storage();
        let before: std::collections::HashSet<_> = list_cloud_storage_entries(&cloud_storage);

        let mut cmd = Command::new(&bundled_binary);
        cmd.arg("mount");
        cmd.arg("--log-file")
            .arg(manager::random_log_file("mount-fp"));
        cmd.arg("--log-level").arg("trace");
        cmd.arg("--unix-socket").arg(ctx.socket_or_pipe());
        cmd.arg("--backend").arg("macos-file-provider");

        for arg in args {
            cmd.arg(arg);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .expect("failed to spawn distant mount (file-provider) process");

        let stdout = child.stdout.take().expect("stdout was not piped");
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("Mounted") {
                    let _ = tx.send(Ok(()));
                    return;
                }
            }
            let _ = tx.send(Err(
                "file-provider mount stdout closed without printing 'Mounted'".to_string(),
            ));
        });

        match rx.recv_timeout(MOUNT_READY_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|s| std::io::read_to_string(s).ok())
                    .unwrap_or_default();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{msg}\nstderr: {stderr}"));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "file-provider mount did not print 'Mounted' within {}s",
                    MOUNT_READY_TIMEOUT.as_secs()
                ));
            }
        }

        // The child exits after printing "Mounted"; reap it now.
        let _ = child.wait();

        let mount_point = discover_cloud_storage_entry(&cloud_storage, &before)?;

        Ok(Self {
            child,
            mount_point,
            is_file_provider: true,
            bundled_bin: Some(bundled_binary),
        })
    }

    /// Spawns a `distant mount --foreground` process and waits for it to be ready.
    ///
    /// # Panics
    ///
    /// Panics if the mount fails to start. Use [`try_spawn`](Self::try_spawn)
    /// for tests that expect the mount to fail.
    pub fn spawn(ctx: &BackendCtx, mount: MountBackend, mount_point: &Path, args: &[&str]) -> Self {
        Self::try_spawn(ctx, mount, mount_point, args)
            .unwrap_or_else(|e| panic!("mount failed: {e}"))
    }

    /// Returns the canonicalized mount point path.
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}

impl Drop for MountProcess {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if self.is_file_provider {
            if let Some(ref bin) = self.bundled_bin {
                let _ = Command::new(bin)
                    .args(["unmount", "--all"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }

            // The child already exited after mounting; reap just in case.
            let _ = self.child.kill();
            let _ = self.child.wait();
            return;
        }

        let mp = self.mount_point.to_string_lossy();

        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("diskutil")
                .args(["unmount", "force", &mp])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        #[cfg(unix)]
        {
            let _ = Command::new("umount")
                .arg("-f")
                .arg(&*mp)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        #[cfg(windows)]
        {
            let bin_path = crate::manager::bin_path();
            let _ = Command::new(&bin_path)
                .args(["unmount", &mp])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        let _ = self.child.kill();
        let _ = self.child.wait();

        wait_for_unmount(&self.mount_point);

        let _ = std::fs::remove_dir_all(&self.mount_point);
    }
}

/// Polls the system `mount` command until the given path no longer appears
/// in its output, or until a 5-second timeout elapses.
///
/// This is useful after issuing an unmount command to wait for the kernel
/// to fully release the mount point before attempting directory cleanup.
pub fn wait_for_unmount(mount_point: &Path) {
    let mp_str = mount_point.to_string_lossy();
    let start = Instant::now();

    while start.elapsed() < UNMOUNT_POLL_TIMEOUT {
        let output = Command::new("mount").output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                if !text.contains(&*mp_str) {
                    return;
                }
            }
            Err(_) => return,
        }
        std::thread::sleep(UNMOUNT_POLL_INTERVAL);
    }
}

/// Poll until a condition is met on the remote, or timeout after 10 seconds.
///
/// Calls `check` every 200ms. Returns `Ok(())` when `check` returns `true`,
/// or panics with `msg` if the timeout expires.
fn poll_until(check: impl Fn() -> bool, msg: &str) {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let interval = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if check() {
            return;
        }
        std::thread::sleep(interval);
    }
    panic!("poll timeout after {}s: {msg}", timeout.as_secs());
}

/// Poll until a remote file exists, or panic after 10 seconds.
pub fn wait_until_exists(ctx: &BackendCtx, path: &str) {
    poll_until(
        || ctx.cli_exists(path),
        &format!("waiting for {path} to exist"),
    );
}

/// Poll until a remote file has the expected content, or panic after 10 seconds.
pub fn wait_until_content(ctx: &BackendCtx, path: &str, expected: &str) {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let interval = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if ctx.cli_exists(path) && ctx.cli_read(path) == expected {
            return;
        }
        std::thread::sleep(interval);
    }

    let actual = if ctx.cli_exists(path) {
        ctx.cli_read(path)
    } else {
        "<file does not exist>".to_string()
    };
    panic!(
        "poll timeout after {}s: waiting for {path} to contain {expected:?}, actual: {actual:?}",
        timeout.as_secs()
    );
}

/// Poll until a remote path no longer exists, or panic after 10 seconds.
pub fn wait_until_gone(ctx: &BackendCtx, path: &str) {
    poll_until(
        || !ctx.cli_exists(path),
        &format!("waiting for {path} to disappear"),
    );
}

/// Deprecated: use [`wait_until_exists`], [`wait_until_content`], or
/// [`wait_until_gone`] instead for polling-based sync verification.
pub fn wait_for_sync() {
    std::thread::sleep(Duration::from_secs(2));
}

/// Force-unmount all stale distant mounts (NFS + FUSE) and poll until
/// the OS mount table is clear. Call before asserting "no mounts found".
pub fn cleanup_all_stale_mounts() {
    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let output = match Command::new("mount").stdout(Stdio::piped()).output() {
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
                let _ = Command::new("umount").arg("-f").arg(path).output();
                #[cfg(target_os = "macos")]
                {
                    let _ = Command::new("diskutil")
                        .args(["unmount", "force"])
                        .arg(path)
                        .output();
                }
            }

            if Instant::now() >= deadline {
                eprintln!("warning: stale mounts still present after 10s: {stale:?}");
                return;
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Builds a test `.app` bundle for the macOS FileProvider backend.
///
/// Creates the directory structure under `target/test-Distant.app/`, copies
/// the distant binary and Info.plist files, signs the bundle with ad-hoc
/// codesign, and registers the FileProvider extension via `pluginkit`.
///
/// Skips the rebuild if the binary modification time has not changed since
/// the last build.
///
/// Returns the path to the `.app` bundle.
///
/// # Panics
///
/// Panics if any filesystem operation, codesign, or pluginkit command fails.
#[cfg(target_os = "macos")]
pub fn build_test_app_bundle() -> PathBuf {
    let workspace_root = find_workspace_root();
    let bin = crate::manager::bin_path();
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));

    let app_dir = target_dir.join("test-Distant.app");
    let contents = app_dir.join("Contents");
    let app_macos = contents.join("MacOS");
    let app_binary = app_macos.join("distant");

    let appex_dir = contents
        .join("PlugIns")
        .join("DistantFileProvider.appex")
        .join("Contents");
    let appex_macos = appex_dir.join("MacOS");
    let appex_binary = appex_macos.join("distant");

    if should_skip_rebuild(&bin, &app_binary) {
        // Even when skipping rebuild, ensure the extension is registered
        // (pluginkit registrations are volatile and may not persist).
        ensure_pluginkit_registered(&contents);
        return app_dir;
    }

    std::fs::create_dir_all(&app_macos).expect("failed to create app MacOS dir");
    std::fs::create_dir_all(&appex_macos).expect("failed to create appex MacOS dir");

    std::fs::copy(&bin, &app_binary).expect("failed to copy binary to app bundle");
    std::fs::copy(&bin, &appex_binary).expect("failed to copy binary to appex bundle");

    let resources = workspace_root.join("resources").join("macos");

    // Use Apple Development identity if available; fall back to ad-hoc.
    let identity = find_apple_dev_identity().unwrap_or_else(|| "-".to_string());

    // Host app: keep the production bundle identifier (dev.distant) so that
    // NSFileProviderManager.addDomain() can find the associated extension.
    // The host binary is just a launcher — macOS identifies it by its bundle ID.
    std::fs::copy(resources.join("Info.plist"), contents.join("Info.plist"))
        .expect("failed to copy Info.plist");

    // Appex: use a distinct bundle identifier but KEEP the production app
    // group. The app group requires a provisioning profile to authorize new
    // groups, so we reuse the existing authorized group. Test and production
    // domains don't collide because domain IDs include unique connection hashes.
    let ext_plist_src = std::fs::read_to_string(resources.join("Extension-Info.plist"))
        .expect("failed to read Extension-Info.plist");

    // Keep the production app group (39C6AGD73Z.group.dev.distant) but
    // change the bundle identifier so macOS treats this as a separate extension.
    let test_app_group = "39C6AGD73Z.group.dev.distant";
    let ext_plist = ext_plist_src.replace(
        "dev.distant.file-provider",
        "dev.distant.test.file-provider",
    );
    std::fs::write(appex_dir.join("Info.plist"), ext_plist)
        .expect("failed to write appex Info.plist");

    let (app_entitlements, appex_entitlements) =
        write_test_entitlements(&target_dir, &test_app_group);

    let appex_path = contents.join("PlugIns").join("DistantFileProvider.appex");
    codesign(&appex_path, &appex_entitlements, &identity);
    codesign(&app_dir, &app_entitlements, &identity);

    ensure_pluginkit_registered(&contents);

    let status = Command::new("pluginkit")
        .args(["-e", "use", "-i", "dev.distant.test.file-provider"])
        .status()
        .expect("failed to run pluginkit -e");
    assert!(status.success(), "pluginkit -e use failed");

    app_dir
}

/// Checks whether the app bundle binary is up-to-date with the source binary.
#[cfg(target_os = "macos")]
fn should_skip_rebuild(source: &Path, dest: &Path) -> bool {
    let src_meta = match std::fs::metadata(source) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dst_meta = match std::fs::metadata(dest) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let src_mtime = src_meta.modified().ok();
    let dst_mtime = dst_meta.modified().ok();
    match (src_mtime, dst_mtime) {
        (Some(s), Some(d)) => s <= d,
        _ => false,
    }
}

/// Writes test entitlements plists for the host app and appex.
///
/// The host app runs unsandboxed so it can access unix sockets freely.
/// The appex must be sandboxed (pluginkit requires it for extensions).
#[cfg(target_os = "macos")]
fn write_test_entitlements(target_dir: &Path, app_group: &str) -> (PathBuf, PathBuf) {
    let app_path = target_dir.join("test-app-entitlements.plist");
    let app_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.application-groups</key>
    <array>
        <string>{app_group}</string>
    </array>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
"#
    );
    std::fs::write(&app_path, app_content).expect("failed to write app entitlements");

    let appex_path = target_dir.join("test-appex-entitlements.plist");
    let appex_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.application-groups</key>
    <array>
        <string>{app_group}</string>
    </array>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
"#
    );
    std::fs::write(&appex_path, appex_content).expect("failed to write appex entitlements");

    (app_path, appex_path)
}

/// Ensures the test FileProvider extension is registered with pluginkit.
///
/// Registrations are volatile and may not persist across reboots, so this
/// is called on every test run (even when the bundle rebuild is skipped).
#[cfg(target_os = "macos")]
fn ensure_pluginkit_registered(contents: &Path) {
    let appex_path = contents.join("PlugIns").join("DistantFileProvider.appex");
    let _ = Command::new("pluginkit")
        .args(["-a", &appex_path.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("pluginkit")
        .args(["-e", "use", "-i", "dev.distant.test.file-provider"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Finds an Apple Development signing identity in the keychain.
///
/// Returns the identity hash string if found, or `None` if no Apple
/// Development certificate is available (ad-hoc signing will be used
/// as a fallback, but pluginkit may reject the extension).
#[cfg(target_os = "macos")]
fn find_apple_dev_identity() -> Option<String> {
    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains("Apple Development:") {
            return line.split_whitespace().nth(1).map(String::from);
        }
    }
    None
}

/// Extracts the Team ID from a signed binary.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn extract_team_id(binary: &Path, identity: &str) -> Option<String> {
    // Sign the binary temporarily to extract team ID
    let _ = Command::new("codesign")
        .args(["-s", identity, "-f"])
        .arg(binary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let output = Command::new("codesign")
        .args(["-dv"])
        .arg(binary)
        .output()
        .ok()?;
    // codesign -dv writes to stderr
    let text = String::from_utf8_lossy(&output.stderr);
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("TeamIdentifier=") {
            if id != "not set" {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Signs a bundle or appex with the given identity (or ad-hoc if `None`).
#[cfg(target_os = "macos")]
fn codesign(path: &Path, entitlements: &Path, identity: &str) {
    let status = Command::new("codesign")
        .args(["-s", identity, "-f", "--entitlements"])
        .arg(entitlements)
        .arg(path)
        .status()
        .expect("failed to run codesign");
    assert!(status.success(), "codesign failed for {}", path.display());
}

/// Finds the workspace root by walking up from `CARGO_MANIFEST_DIR` until
/// a `Cargo.toml` containing `[workspace]` is found.
#[cfg(target_os = "macos")]
fn find_workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists()
            && let Ok(content) = std::fs::read_to_string(&cargo_toml)
            && content.contains("[workspace]")
        {
            return dir;
        }
        if !dir.pop() {
            panic!(
                "could not find workspace root from {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

/// Returns the `~/Library/CloudStorage/` directory path.
#[cfg(target_os = "macos")]
fn dirs_cloud_storage() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
        .join("Library")
        .join("CloudStorage")
}

/// Lists the current entries in the CloudStorage directory.
#[cfg(target_os = "macos")]
fn list_cloud_storage_entries(dir: &Path) -> std::collections::HashSet<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => std::collections::HashSet::new(),
    }
}

/// Discovers a newly-appeared entry in `~/Library/CloudStorage/` by comparing
/// the current listing against a snapshot taken before the mount.
///
/// Polls for up to [`MOUNT_READY_TIMEOUT`] to handle potential delay between
/// the process printing "Mounted" and the directory appearing on disk.
#[cfg(target_os = "macos")]
fn discover_cloud_storage_entry(
    cloud_storage: &Path,
    before: &std::collections::HashSet<PathBuf>,
) -> Result<PathBuf, String> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(200);

    while start.elapsed() < MOUNT_READY_TIMEOUT {
        let current = list_cloud_storage_entries(cloud_storage);
        let new_entries: Vec<_> = current.difference(before).collect();

        if let Some(entry) = new_entries.into_iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("Distant"))
        }) {
            return Ok(entry.clone());
        }

        std::thread::sleep(poll_interval);
    }

    Err(format!(
        "no new Distant entry appeared in {} within {}s",
        cloud_storage.display(),
        MOUNT_READY_TIMEOUT.as_secs()
    ))
}

/// Template for testing all plugin backends (host, ssh, docker).
#[template]
#[export]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[cfg_attr(feature = "docker", case::docker(Backend::Docker))]
pub fn all_plugins(#[case] backend: Backend) {}

/// Template for testing all combinations of plugin backends and mount backends.
#[template]
#[export]
#[rstest]
#[cfg_attr(
    feature = "mount-nfs",
    case::host_nfs(Backend::Host, MountBackend::Nfs)
)]
#[cfg_attr(feature = "mount-nfs", case::ssh_nfs(Backend::Ssh, MountBackend::Nfs))]
#[cfg_attr(
    all(feature = "docker", feature = "mount-nfs"),
    case::docker_nfs(Backend::Docker, MountBackend::Nfs)
)]
#[cfg_attr(
    all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ),
    case::host_fuse(Backend::Host, MountBackend::Fuse)
)]
#[cfg_attr(
    all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ),
    case::ssh_fuse(Backend::Ssh, MountBackend::Fuse)
)]
#[cfg_attr(
    all(feature = "mount-windows-cloud-files", target_os = "windows"),
    case::host_wcf(Backend::Host, MountBackend::WindowsCloudFiles)
)]
#[cfg_attr(
    all(feature = "mount-macos-file-provider", target_os = "macos"),
    case::host_fp(Backend::Host, MountBackend::MacosFileProvider)
)]
pub fn plugin_x_mount(#[case] backend: Backend, #[case] mount: MountBackend) {}
