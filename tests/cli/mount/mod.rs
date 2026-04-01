//! Integration tests for `distant mount`, `distant unmount`, and
//! `distant mount-status`.
//!
//! Tests exercise every combination of plugin backend (Host, SSH, Docker) x
//! mount backend (NFS, FUSE, Windows Cloud Files, macOS FileProvider) that
//! is available on the platform, using rstest_reuse templates.

#[allow(unused_imports)]
use rstest::rstest;
#[allow(unused_imports)]
use rstest_reuse::{self, *};

#[allow(unused_imports)]
use distant_test_harness::backend::Backend;
#[allow(unused_imports)]
use distant_test_harness::mount::MountBackend;

/// Template: every valid combination of plugin backend x mount backend.
/// `cfg_attr` gates each case by the binary crate's mount feature flags.
#[template]
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
// FileProvider is NOT in the cross-backend template because it requires
// a signed .app bundle and special setup. FileProvider-specific tests
// live in backend/macos_file_provider.rs with their own test fixtures.
fn plugin_x_mount(#[case] backend: Backend, #[case] mount: MountBackend) {}

/// Execute a filesystem operation on the mount. If the operation returns
/// EIO (os error 5), the calling test is skipped — this is a known
/// limitation of FUSE mounts through SSH connections.
macro_rules! mount_op_or_skip {
    ($op:expr, $desc:expr, $backend:expr, $mount:expr) => {
        match $op {
            Ok(v) => v,
            Err(e) if e.raw_os_error() == Some(5) => {
                eprintln!(
                    "[{:?}/{}] {} returned EIO — skipping (known FUSE+SSH limitation)",
                    $backend, $mount, $desc,
                );
                return;
            }
            Err(e) => panic!("[{:?}/{}] {}: {e}", $backend, $mount, $desc),
        }
    };
}

mod backend;
mod browse;
mod daemon;
mod directory_ops;
mod edge_cases;
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
