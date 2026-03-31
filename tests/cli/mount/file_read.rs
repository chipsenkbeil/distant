//! Integration tests for reading file contents through a mounted filesystem.
//!
//! Verifies that regular files, large files, and missing files are handled
//! correctly when accessed via standard filesystem operations on the mount point.

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// FRD-01: Reading a seeded file through the mount should return its exact contents.
#[rstest]
#[test_log::test]
fn read_should_return_file_contents(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let contents = std::fs::read_to_string(mount.mount_point().join("hello.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read hello.txt: {e}"));

        assert_eq!(
            contents, "hello world",
            "[{backend}] hello.txt contents mismatch"
        );
    }
}

/// FRD-02: A large file (100 KB) written via `distant fs write` should be
/// readable through the mount with matching contents.
#[rstest]
#[test_log::test]
fn read_should_handle_large_file(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    let large_content: String = "A".repeat(100 * 1024);
    let large_file_path = seed_dir.path().join("large.txt");

    ctx.new_assert_cmd(["fs", "write"])
        .args([large_file_path.to_str().unwrap()])
        .write_stdin(large_content.as_str())
        .assert()
        .success();

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let contents = std::fs::read_to_string(mount.mount_point().join("large.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read large.txt: {e}"));

        assert_eq!(
            contents.len(),
            large_content.len(),
            "[{backend}] large file size mismatch"
        );
        assert_eq!(
            contents, large_content,
            "[{backend}] large file contents mismatch"
        );
    }
}

/// FRD-03: Reading a file that does not exist on the remote should return
/// a filesystem error.
#[rstest]
#[test_log::test]
fn read_should_fail_for_nonexistent_file(ctx: ManagerCtx) {
    let seed_dir = assert_fs::TempDir::new().unwrap();
    seed_test_data(&ctx, seed_dir.path());

    for backend in available_backends() {
        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &["--remote-root", seed_dir.path().to_str().unwrap()],
        );

        let result = std::fs::read_to_string(mount.mount_point().join("nonexistent.txt"));

        assert!(
            result.is_err(),
            "[{backend}] reading nonexistent file should fail, but got: {:?}",
            result.unwrap()
        );
    }
}
