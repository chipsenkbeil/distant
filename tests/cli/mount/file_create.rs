//! Integration tests for creating files through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// FCR-01: Writing a new file at the mount point root should propagate to
/// the remote directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn create_file_should_appear_on_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-create-file");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    mount_op_or_skip!(
        std::fs::write(mp.mount_point().join("new.txt"), "created"),
        "write new.txt",
        backend,
        mount
    );

    distant_test_harness::mount::wait_for_sync();

    let remote_path = ctx.child_path(&dir, "new.txt");
    assert!(
        ctx.cli_exists(&remote_path),
        "[{backend:?}/{mount}] new.txt should exist on remote"
    );
    assert_eq!(
        ctx.cli_read(&remote_path),
        "created",
        "[{backend:?}/{mount}] remote content mismatch"
    );
}

/// FCR-02: Writing a new file inside a subdirectory of the mount should
/// propagate to the corresponding remote subdirectory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn create_file_in_subdir_should_appear_on_remote(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-create-subdir");
    ctx.cli_mkdir(&dir);
    ctx.cli_mkdir(&ctx.child_path(&dir, "subdir"));

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    mount_op_or_skip!(
        std::fs::write(
            mp.mount_point().join("subdir").join("new.txt"),
            "sub-created"
        ),
        "write subdir/new.txt",
        backend,
        mount
    );

    distant_test_harness::mount::wait_for_sync();

    let remote_path = ctx.child_path(&ctx.child_path(&dir, "subdir"), "new.txt");
    assert!(
        ctx.cli_exists(&remote_path),
        "[{backend:?}/{mount}] subdir/new.txt should exist on remote"
    );
    assert_eq!(
        ctx.cli_read(&remote_path),
        "sub-created",
        "[{backend:?}/{mount}] remote subdir content mismatch"
    );
}
