//! Integration tests for `--remote-root` option behavior.

use std::collections::HashSet;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// RRT-01: Mounting with `--remote-root` pointing to a subdirectory should
/// expose only the contents of that subdirectory at the mount point root.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn remote_root_should_scope_to_subdir(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-rroot-scope");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "parent.txt"), "in parent");

    let subdir = ctx.child_path(&dir, "child");
    ctx.cli_mkdir(&subdir);
    ctx.cli_write(&ctx.child_path(&subdir, "child.txt"), "in child");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &subdir]);

    let entries: HashSet<String> = std::fs::read_dir(mp.mount_point())
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read mount point: {e}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.contains("child.txt"),
        "[{backend:?}/{mount}] expected child.txt in scoped mount, got: {entries:?}"
    );
    assert!(
        !entries.contains("parent.txt"),
        "[{backend:?}/{mount}] parent.txt should NOT appear in scoped mount, got: {entries:?}"
    );
}

/// RRT-02: Mounting with `--remote-root` pointing to a nonexistent path should
/// either fail at mount time or produce errors when the filesystem is accessed.
/// NFS validates the root path at mount time (fails to start). FUSE defers
/// validation until access time (mounts successfully, but reads fail).
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn remote_root_nonexistent_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let bogus = ctx.unique_dir("mount-rroot-nonexistent");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let result = MountProcess::try_spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &bogus]);

    match result {
        Err(_) => {
            // Mount failed at startup (expected for NFS)
        }
        Ok(mp) => {
            // Mount succeeded but filesystem access should fail (FUSE)
            let read_result = std::fs::read_dir(mp.mount_point());
            assert!(
                read_result.is_err() || read_result.unwrap().count() == 0,
                "[{backend:?}/{mount}] nonexistent remote root should produce empty or error on access"
            );
        }
    }
}
