//! Integration tests for creating files through a mounted directory.

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// FCR-01: Writing a new file at the mount point root should propagate to
/// the remote directory.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn create_file_should_appear_on_remote(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-create-file");

    std::fs::write(sm.mount_point.join(&subdir_name).join("new.txt"), "created")
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to write new.txt: {e}"));

    let remote_path = ctx.child_path(&subdir, "new.txt");
    mount::wait_until_content(&ctx, &remote_path, "created");

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

    let sm = mount::get_or_start_mount(&ctx, mount);
    let (subdir, subdir_name) = mount::unique_subdir(&ctx, &sm.remote_root, "mount-create-subdir");
    ctx.cli_mkdir(&ctx.child_path(&subdir, "subdir"));

    std::fs::write(
        sm.mount_point
            .join(&subdir_name)
            .join("subdir")
            .join("new.txt"),
        "sub-created",
    )
    .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to write subdir/new.txt: {e}"));

    let remote_path = ctx.child_path(&ctx.child_path(&subdir, "subdir"), "new.txt");
    mount::wait_until_content(&ctx, &remote_path, "sub-created");

    assert_eq!(
        ctx.cli_read(&remote_path),
        "sub-created",
        "[{backend:?}/{mount}] remote subdir content mismatch"
    );
}
