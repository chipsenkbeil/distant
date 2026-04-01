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
pub struct MountProcess {
    child: Child,
    mount_point: PathBuf,
}

impl MountProcess {
    /// Spawns a `distant mount --foreground` process and waits for it to be ready.
    ///
    /// Uses `ctx.new_std_cmd(["mount"])` to get a properly configured command,
    /// then adds `--backend`, `--foreground`, any extra `args`, and the
    /// `mount_point`. Blocks until the process prints "Mounted" on stdout
    /// (up to 30 seconds).
    ///
    /// After mount succeeds, the mount point is canonicalized to resolve
    /// macOS `/var` to `/private/var` symlinks.
    ///
    /// # Panics
    ///
    /// Panics if the process fails to spawn, does not print "Mounted" within
    /// the timeout, or if the mount point cannot be canonicalized.
    pub fn spawn(ctx: &BackendCtx, mount: MountBackend, mount_point: &Path, args: &[&str]) -> Self {
        std::fs::create_dir_all(mount_point).expect("failed to create mount point directory");

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
                "mount process stdout closed without printing 'Mounted'",
            ));
        });

        match rx.recv_timeout(MOUNT_READY_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => panic!("mount failed: {msg}"),
            Err(_) => {
                let _ = child.kill();
                panic!(
                    "mount process did not print 'Mounted' within {}s",
                    MOUNT_READY_TIMEOUT.as_secs()
                );
            }
        }

        let canonical = std::fs::canonicalize(mount_point).unwrap_or_else(|e| {
            panic!(
                "failed to canonicalize mount point {}: {e}",
                mount_point.display()
            )
        });

        Self {
            child,
            mount_point: canonical,
        }
    }

    /// Returns the canonicalized mount point path.
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}

impl Drop for MountProcess {
    fn drop(&mut self) {
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
        return app_dir;
    }

    std::fs::create_dir_all(&app_macos).expect("failed to create app MacOS dir");
    std::fs::create_dir_all(&appex_macos).expect("failed to create appex MacOS dir");

    std::fs::copy(&bin, &app_binary).expect("failed to copy binary to app bundle");
    std::fs::copy(&bin, &appex_binary).expect("failed to copy binary to appex bundle");

    let resources = workspace_root.join("resources").join("macos");
    std::fs::copy(resources.join("Info.plist"), contents.join("Info.plist"))
        .expect("failed to copy Info.plist");

    let ext_plist_src = std::fs::read_to_string(resources.join("Extension-Info.plist"))
        .expect("failed to read Extension-Info.plist");
    let ext_plist = ext_plist_src.replace("39C6AGD73Z.group.dev.distant", "group.dev.distant.test");
    std::fs::write(appex_dir.join("Info.plist"), ext_plist)
        .expect("failed to write appex Info.plist");

    let entitlements = write_test_entitlements(&target_dir);

    let appex_path = contents.join("PlugIns").join("DistantFileProvider.appex");
    codesign(&appex_path, &entitlements);
    codesign(&app_dir, &entitlements);

    let status = Command::new("pluginkit")
        .args(["-a", &app_dir.to_string_lossy()])
        .status()
        .expect("failed to run pluginkit -a");
    assert!(status.success(), "pluginkit -a failed");

    let status = Command::new("pluginkit")
        .args(["-e", "use", "-i", "dev.distant.file-provider"])
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

/// Writes a minimal test entitlements plist to a temp file and returns its path.
#[cfg(target_os = "macos")]
fn write_test_entitlements(target_dir: &Path) -> PathBuf {
    let path = target_dir.join("test-entitlements.plist");
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
"#;
    std::fs::write(&path, content).expect("failed to write test entitlements");
    path
}

/// Signs a bundle or appex with ad-hoc codesign.
#[cfg(target_os = "macos")]
fn codesign(path: &Path, entitlements: &Path) {
    let status = Command::new("codesign")
        .args(["-s", "-", "-f", "--entitlements"])
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
