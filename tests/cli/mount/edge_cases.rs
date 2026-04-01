//! Integration tests for mount edge cases and robustness.

use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::mount::{self, MountBackend, MountProcess};
use distant_test_harness::skip_if_no_backend;

/// EDG-01: Mounting to a directory that does not yet exist should auto-create it.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mount_should_auto_create_directory(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-edge-autocreate");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let tmp = assert_fs::TempDir::new().unwrap();
    let mount_point = tmp.path().join("nonexistent-sub");

    assert!(
        !mount_point.exists(),
        "mount point should not exist before spawn"
    );

    let mp = MountProcess::spawn(&ctx, mount, &mount_point, &["--remote-root", &dir]);

    let content = std::fs::read_to_string(mp.mount_point().join("probe.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read probe.txt: {e}"));

    assert_eq!(
        content, "probe",
        "[{backend:?}/{mount}] content mismatch after auto-create"
    );
}

/// EDG-02: Mounting onto a path that is a regular file should fail.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn mount_onto_file_should_fail(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-edge-file");
    ctx.cli_mkdir(&dir);

    let tmp = assert_fs::TempDir::new().unwrap();
    let file_path = tmp.path().join("regular-file");
    std::fs::write(&file_path, "i am a file").expect("failed to create regular file");

    let result = MountProcess::try_spawn(&ctx, mount, &file_path, &["--remote-root", &dir]);

    assert!(
        result.is_err(),
        "[{backend:?}/{mount}] mounting onto a regular file should fail"
    );
}

/// EDG-03: Filenames containing spaces should be accessible through the mount.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn filenames_with_spaces_should_work(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-edge-spaces");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello world.txt"), "space content");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    let content = std::fs::read_to_string(mp.mount_point().join("hello world.txt"))
        .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] failed to read 'hello world.txt': {e}"));

    assert_eq!(
        content, "space content",
        "[{backend:?}/{mount}] content mismatch for filename with spaces"
    );
}

/// EDG-04: Rapid sequential write/read cycles should not corrupt data.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn rapid_write_read_should_not_corrupt(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-edge-rapid");
    ctx.cli_mkdir(&dir);

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    for i in 0..10 {
        let name = format!("rapid-{i}.txt");
        let content = format!("iteration-{i}");
        let path = mp.mount_point().join(&name);

        std::fs::write(&path, &content)
            .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] write failed on iteration {i}: {e}"));

        std::thread::sleep(Duration::from_millis(50));

        let read_back = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("[{backend:?}/{mount}] read failed on iteration {i}: {e}"));

        assert_eq!(
            read_back, content,
            "[{backend:?}/{mount}] data corruption on iteration {i}"
        );
    }
}

/// EDG-05: After dropping a `MountProcess`, no stale mount should remain in
/// the system mount table.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn drop_should_leave_no_stale_mounts(
    #[case] backend: Backend,
    #[case] mount_backend: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-edge-stale");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "canary.txt"), "canary");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mount_path = mount_dir.path().to_path_buf();

    {
        let _mp = MountProcess::spawn(&ctx, mount_backend, &mount_path, &["--remote-root", &dir]);
    }

    mount::wait_for_unmount(&mount_path);

    let output = std::process::Command::new("mount")
        .output()
        .expect("failed to run mount command");

    let mount_table = String::from_utf8_lossy(&output.stdout);
    let mp_str = mount_path.to_string_lossy();

    assert!(
        !mount_table.contains(&*mp_str),
        "[{backend:?}/{mount_backend}] mount table should not contain stale entry for '{mp_str}'"
    );
}
