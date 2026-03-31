//! Integration tests for read-only mount behavior.
//!
//! Verifies that mounting with `--readonly` allows read operations but
//! prevents write and delete operations.

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// RDO-01: A read-only mount should allow reading files.
#[rstest]
#[test_log::test]
fn readonly_mount_should_allow_reads(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &[
                "--remote-root",
                seed_dir.path().to_str().unwrap(),
                "--readonly",
            ],
        );

        let contents = std::fs::read_to_string(mount.mount_point().join("hello.txt"))
            .unwrap_or_else(|e| panic!("[{backend}] read should succeed on readonly mount: {e}"));

        assert_eq!(
            contents, "hello world",
            "[{backend}] readonly mount read content mismatch"
        );
    }
}

/// RDO-02: A read-only mount should block file creation.
#[rstest]
#[test_log::test]
fn readonly_mount_should_block_writes(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &[
                "--remote-root",
                seed_dir.path().to_str().unwrap(),
                "--readonly",
            ],
        );

        let result = std::fs::write(mount.mount_point().join("new.txt"), "data");
        assert!(
            result.is_err(),
            "[{backend}] writing to readonly mount should fail"
        );
    }
}

/// RDO-03: A read-only mount should block file deletion.
#[rstest]
#[test_log::test]
fn readonly_mount_should_block_deletes(ctx: ManagerCtx) {
    for backend in available_backends() {
        let seed_dir = assert_fs::TempDir::new().unwrap();
        seed_test_data(&ctx, seed_dir.path());

        let mount_dir = assert_fs::TempDir::new().unwrap();
        let mount = MountProcess::spawn(
            &ctx,
            backend,
            mount_dir.path(),
            &[
                "--remote-root",
                seed_dir.path().to_str().unwrap(),
                "--readonly",
            ],
        );

        let result = std::fs::remove_file(mount.mount_point().join("hello.txt"));
        assert!(
            result.is_err(),
            "[{backend}] deleting from readonly mount should fail"
        );
    }
}
