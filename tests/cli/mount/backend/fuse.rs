//! FUSE-specific mount integration tests.

use rstest::rstest;

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// BKE-FUSE: While a FUSE mount is active, the system `mount` command output
/// should contain a `fuse` or `macfuse` reference for the mount point.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn fuse_mount_should_appear_in_mount_table(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-bke-fuse");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(
        &ctx,
        MountBackend::Fuse,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let output = std::process::Command::new("mount")
        .output()
        .expect("failed to run mount command");

    let mount_table = String::from_utf8_lossy(&output.stdout);
    let mp_str = mp.mount_point().to_string_lossy();

    assert!(
        mount_table.contains(&*mp_str),
        "[{backend:?}/fuse] mount point should appear in mount table"
    );

    let fuse_line = mount_table
        .lines()
        .find(|line| line.contains(&*mp_str))
        .expect("mount table should contain a line with the mount point");

    let lower = fuse_line.to_lowercase();
    assert!(
        lower.contains("fuse") || lower.contains("macfuse"),
        "[{backend:?}/fuse] mount table entry should reference fuse/macfuse, got: {fuse_line}"
    );
}
