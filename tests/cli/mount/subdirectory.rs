//! Integration tests for navigating subdirectories within a mounted filesystem.
//!
//! Verifies that subdirectory listing and deeply nested file reads work
//! correctly through the OS filesystem layer backed by the mount.

use std::collections::HashSet;

use rstest::*;

use distant_test_harness::manager::*;

use super::*;

/// SDT-01: Listing a subdirectory through the mount should return its
/// immediate children.
#[rstest]
#[test_log::test]
fn subdir_should_list_contents(ctx: ManagerCtx) {
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

        let entries: HashSet<String> = std::fs::read_dir(mount.mount_point().join("subdir"))
            .unwrap_or_else(|e| panic!("[{backend}] failed to read subdir: {e}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert!(
            entries.contains("nested.txt"),
            "[{backend}] expected nested.txt in subdir, got: {entries:?}"
        );
        assert!(
            entries.contains("deep"),
            "[{backend}] expected deep in subdir, got: {entries:?}"
        );
        assert_eq!(
            entries.len(),
            2,
            "[{backend}] subdir should contain exactly 2 entries, got: {entries:?}"
        );
    }
}

/// SDT-02: A file nested several levels deep should be readable through
/// the mount with correct contents.
#[rstest]
#[test_log::test]
fn deeply_nested_file_should_be_readable(ctx: ManagerCtx) {
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

        let path = mount
            .mount_point()
            .join("subdir")
            .join("deep")
            .join("deeper.txt");
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("[{backend}] failed to read deeper.txt: {e}"));

        assert_eq!(
            contents, "deep content",
            "[{backend}] deeper.txt contents mismatch"
        );
    }
}
