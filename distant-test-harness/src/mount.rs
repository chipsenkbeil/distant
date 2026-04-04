//! Mount test infrastructure for integration tests.
//!
//! Provides [`MountProcess`] for managing foreground mount lifecycles,
//! [`wait_for_unmount`] for polling unmount completion, and
//! [`build_test_app_bundle`] for constructing macOS FileProvider `.app`
//! bundles in tests. Also re-exports [`MountBackend`] for convenience
//! and defines rstest-reuse templates for parameterized mount tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
/// cleanup (unmount via manager + directory removal) on drop.
pub struct MountProcess {
    /// Mount ID returned by the manager.
    mount_id: Option<u32>,
    /// Socket path for sending unmount commands.
    socket_or_pipe: String,
    mount_point: PathBuf,

    /// When true, this mount was created via the macOS FileProvider backend.
    #[allow(dead_code)]
    is_file_provider: bool,

    /// Path to the `.app` bundle binary used for FileProvider mounts.
    #[allow(dead_code)]
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

    /// Mounts via the manager. The CLI sends a mount request to the manager,
    /// prints the result, and exits immediately.
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
        cmd.arg("--backend").arg(mount.as_str());

        for arg in args {
            cmd.arg(arg);
        }

        cmd.arg(mount_point);

        let output = cmd
            .output()
            .map_err(|e| format!("failed to run distant mount: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("mount failed: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse mount ID from output: "Mounted at /path (id: 123)"
        let mount_id = stdout
            .lines()
            .find(|l| l.contains("Mounted"))
            .and_then(|l| l.rsplit("id: ").next())
            .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

        let canonical = std::fs::canonicalize(mount_point).map_err(|e| {
            format!(
                "failed to canonicalize mount point {}: {e}",
                mount_point.display()
            )
        })?;

        Ok(Self {
            mount_id,
            socket_or_pipe: ctx.socket_or_pipe().to_string(),
            mount_point: canonical,
            is_file_provider: false,
            #[cfg(target_os = "macos")]
            bundled_bin: None,
        })
    }

    /// Mounts via the FileProvider backend using the installed `.app` binary.
    ///
    /// The command sends a mount request through the FP singleton's manager,
    /// prints the result, and exits immediately. The mount point is
    /// discovered by scanning `~/Library/CloudStorage/` for new entries.
    #[cfg(target_os = "macos")]
    fn try_spawn_file_provider(_ctx: &BackendCtx, args: &[&str]) -> Result<Self, String> {
        use crate::{manager, singleton};

        let fp_handle = singleton::get_or_start_file_provider();

        let bin = PathBuf::from("/Applications/Distant.app/Contents/MacOS/distant");

        let cloud_storage = dirs_cloud_storage();
        let before: std::collections::HashSet<_> = list_cloud_storage_entries(&cloud_storage);

        let mut cmd = Command::new(&bin);
        cmd.arg("mount");
        cmd.arg("--log-file")
            .arg(manager::random_log_file("mount-fp"));
        cmd.arg("--log-level").arg("trace");
        cmd.arg("--unix-socket").arg(&fp_handle.socket_or_pipe);
        cmd.arg("--backend").arg("macos-file-provider");

        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("failed to run distant mount (file-provider): {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("file-provider mount failed:\nstderr: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mount_id = stdout
            .lines()
            .find(|l| l.contains("Mounted"))
            .and_then(|l| l.rsplit("id: ").next())
            .and_then(|s| s.trim_end_matches(')').parse::<u32>().ok());

        let mount_point = discover_cloud_storage_entry(&cloud_storage, &before)?;

        Ok(Self {
            mount_id,
            socket_or_pipe: fp_handle.socket_or_pipe.clone(),
            mount_point,
            is_file_provider: true,
            #[cfg(target_os = "macos")]
            bundled_bin: Some(bin),
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
        // Unmount via manager if we have a mount ID
        if let Some(id) = self.mount_id {
            let mut cmd = Command::new(crate::manager::bin_path());
            cmd.arg("unmount").arg(id.to_string());

            if cfg!(windows) {
                cmd.arg("--windows-pipe").arg(&self.socket_or_pipe);
            } else {
                cmd.arg("--unix-socket").arg(&self.socket_or_pipe);
            }

            let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
        }

        // Safety net: force unmount via OS if the mount point is still active
        #[cfg(target_os = "macos")]
        {
            let mp = self.mount_point.to_string_lossy();
            let _ = Command::new("diskutil")
                .args(["unmount", "force", &mp])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mp = self.mount_point.to_string_lossy();
            let _ = Command::new("umount")
                .arg("-f")
                .arg(&*mp)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

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

/// Installs the test `.app` bundle to `/Applications/Distant.app`.
///
/// Backs up any existing production install to `/Applications/Distant.app.bak`.
/// Uses `scripts/build-macos-app.sh --skip-build` to bundle, sign, and install.
/// Requires an Apple Development signing identity in the keychain.
///
/// # Errors
///
/// Returns `Err` if signing identity is missing, the script fails, or
/// `/Applications/` is not writable.
#[cfg(target_os = "macos")]
pub fn install_test_app() -> Result<(), String> {
    let workspace = find_workspace_root();
    let app = PathBuf::from("/Applications/Distant.app");
    let backup = PathBuf::from("/Applications/Distant.app.bak");

    // Back up existing production install (only if not already backed up)
    if app.exists() && !backup.exists() {
        std::fs::rename(&app, &backup)
            .map_err(|e| format!("failed to back up /Applications/Distant.app: {e}"))?;
    }

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    let status = Command::new("bash")
        .arg(workspace.join("scripts/build-macos-app.sh"))
        .arg("--skip-build")
        .env("CARGO_PROFILE", profile)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| format!("failed to run build-macos-app.sh: {e}"))?;

    if !status.success() {
        // Restore backup on failure
        restore_production_app();
        return Err(
            "build-macos-app.sh failed (missing Apple Development signing identity?)".into(),
        );
    }

    Ok(())
}

/// Restores the production `/Applications/Distant.app` from backup.
///
/// Called after tests complete or if install fails. Re-registers the
/// restored extension with pluginkit.
#[cfg(target_os = "macos")]
pub fn restore_production_app() {
    let app = PathBuf::from("/Applications/Distant.app");
    let backup = PathBuf::from("/Applications/Distant.app.bak");

    let _ = std::fs::remove_dir_all(&app);

    if backup.exists() {
        let _ = std::fs::rename(&backup, &app);
        // Re-register the restored extension
        let appex = app
            .join("Contents")
            .join("PlugIns")
            .join("DistantFileProvider.appex");
        let _ = Command::new("pluginkit")
            .args(["-a", &appex.to_string_lossy()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pluginkit")
            .args(["-e", "use", "-i", "dev.distant.file-provider"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
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
