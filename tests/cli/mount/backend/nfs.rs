//! NFS-specific mount integration tests.

use rstest::rstest;

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// BKE-NFS: While an NFS mount is active, the system `mount` command output
/// should contain an `nfs` reference for the mount point.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn nfs_mount_should_appear_in_mount_table(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, MountBackend::Nfs);

    let output = std::process::Command::new("mount")
        .output()
        .expect("failed to run mount command");

    let mount_table = String::from_utf8_lossy(&output.stdout);
    let mp_str = sm.mount_point.to_string_lossy();

    assert!(
        mount_table.contains(&*mp_str),
        "[{backend:?}/nfs] mount point should appear in mount table"
    );

    let nfs_line = mount_table
        .lines()
        .find(|line| line.contains(&*mp_str))
        .expect("mount table should contain a line with the mount point");

    assert!(
        nfs_line.to_lowercase().contains("nfs"),
        "[{backend:?}/nfs] mount table entry should reference nfs, got: {nfs_line}"
    );
}
