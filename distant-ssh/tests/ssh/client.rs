#![allow(unexpected_cfgs)] // Allow ci cfg condition used in test attributes

use std::io;
use std::path::Path;
use std::time::Duration;

use assert_fs::TempDir;
use assert_fs::prelude::*;
use distant_core::protocol::{
    ChangeKindSet, Environment, FileType, Metadata, Permissions, PtySize, SearchQuery,
    SearchQueryCondition, SearchQueryTarget, SetPermissionsOptions,
};
use distant_core::{ChannelExt, Client};
use once_cell::sync::Lazy;
use predicates::prelude::*;
use rstest::*;
use test_log::test;

use distant_test_harness::sshd::*;

/// Returns a platform-appropriate command string.
/// On Unix, uses the unix_cmd; on Windows, uses the windows_cmd.
fn platform_cmd(unix_cmd: &str, windows_cmd: &str) -> String {
    if cfg!(windows) {
        windows_cmd.to_string()
    } else {
        unix_cmd.to_string()
    }
}

static TEMP_SCRIPT_DIR: Lazy<TempDir> = Lazy::new(|| TempDir::new().unwrap());

static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

#[rstest]
#[test(tokio::test)]
async fn read_file_should_fail_if_file_missing(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let path = temp.child("missing-file").path().to_path_buf();

    let _ = client.read_file(path).await.unwrap_err();
}

#[rstest]
#[test(tokio::test)]
async fn read_file_should_send_blob_with_file_contents(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let bytes = client.read_file(file.path().to_path_buf()).await.unwrap();
    assert_eq!(bytes, b"some file contents");
}

#[rstest]
#[test(tokio::test)]
async fn append_file_text_should_send_error_if_fails_to_create_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("nonexistent-dir").child("test-file");

    // Ensure that it doesn't exist and we get an error
    let _ = client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap_err();

    // Also, verify that the file doesn't exist
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn read_file_text_should_send_text_with_file_contents(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let text = client
        .read_file_text(file.path().to_path_buf())
        .await
        .unwrap();
    assert_eq!(text, "some file contents");
}

#[rstest]
#[test(tokio::test)]
async fn write_file_should_send_error_if_fails_to_write_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let _ = client
        .write_file(file.path().to_path_buf(), b"some text".to_vec())
        .await
        .unwrap_err();

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn write_file_should_send_ok_when_successful(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Path should point to a file that does not exist, but all
    // other components leading up to it do
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .write_file(file.path().to_path_buf(), b"some text".to_vec())
        .await
        .unwrap();

    // Also verify that we actually did create the file
    // with the associated contents
    file.assert("some text");
}

#[rstest]
#[test(tokio::test)]
async fn write_file_text_should_send_error_if_fails_to_write_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let _ = client
        .write_file_text(file.path().to_path_buf(), "some text".to_string())
        .await
        .unwrap_err();

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn write_file_text_should_send_ok_when_successful(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Path should point to a file that does not exist, but all
    // other components leading up to it do
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .write_file_text(file.path().to_path_buf(), "some text".to_string())
        .await
        .unwrap();

    // Also verify that we actually did create the file
    // with the associated contents
    file.assert("some text");
}

#[rstest]
#[test(tokio::test)]
async fn append_file_should_send_error_if_fails_to_create_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let _ = client
        .append_file(file.path().to_path_buf(), b"some extra contents".to_vec())
        .await
        .unwrap_err();

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn append_file_should_create_file_if_missing(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Don't create the file directly, but define path
    // where the file should be
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .append_file(file.path().to_path_buf(), b"some extra contents".to_vec())
        .await
        .unwrap();

    // Also verify that we actually did create to the file
    file.assert("some extra contents");
}

#[rstest]
#[test(tokio::test)]
async fn append_file_should_send_ok_when_successful(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary file and fill it with some contents
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    client
        .append_file(file.path().to_path_buf(), b"some extra contents".to_vec())
        .await
        .unwrap();

    // Give SFTP a moment to flush on Windows
    #[cfg(windows)]
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[test(tokio::test)]
async fn append_file_text_should_send_error_if_parent_directory_missing(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let _ = client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap_err();

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn append_file_text_should_create_file_if_missing(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Don't create the file directly, but define path
    // where the file should be
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap();

    // Also verify that we actually did create to the file
    file.assert("some extra contents");
}

#[rstest]
#[test(tokio::test)]
async fn append_file_text_should_send_ok_when_successful(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create a temporary file and fill it with some contents
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap();

    // Give SFTP a moment to flush on Windows
    #[cfg(windows)]
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[test(tokio::test)]
async fn dir_read_should_send_error_if_directory_does_not_exist(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("test-dir");

    let _ = client
        .read_dir(
            dir.path().to_path_buf(),
            /* depth */ 0,
            /* absolute */ false,
            /* canonicalize */ false,
            /* include_root */ false,
        )
        .await
        .unwrap_err();
}

// /root/
// /root/file1
// /root/link1 -> /root/sub1/file2
// /root/sub1/
// /root/sub1/file2
async fn setup_dir() -> assert_fs::TempDir {
    let root_dir = assert_fs::TempDir::new().unwrap();

    let file1 = root_dir.child("file1");
    file1.touch().unwrap();

    let sub1 = root_dir.child("sub1");
    sub1.create_dir_all().unwrap();

    let file2 = sub1.child("file2");
    file2.touch().unwrap();

    let link1 = root_dir.child("link1");
    link1.symlink_to_file(file2.path()).unwrap();

    root_dir
}

// NOTE: CI fails this on Windows, but it's running Windows with bash and strange paths, so ignore
//       it only for the CI
#[rstest]
#[test(tokio::test)]
#[cfg_attr(all(windows, ci), ignore)]
async fn dir_read_should_support_depth_limits(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 1,
            /* absolute */ false,
            /* canonicalize */ false,
            /* include_root */ false,
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 3, "Wrong number of entries found");

    assert_eq!(entries[0].file_type, FileType::File);
    assert_eq!(entries[0].path, Path::new("file1"));
    assert_eq!(entries[0].depth, 1);

    assert_eq!(entries[1].file_type, FileType::Symlink);
    assert_eq!(entries[1].path, Path::new("link1"));
    assert_eq!(entries[1].depth, 1);

    assert_eq!(entries[2].file_type, FileType::Dir);
    assert_eq!(entries[2].path, Path::new("sub1"));
    assert_eq!(entries[2].depth, 1);
}

// NOTE: CI fails this on Windows, but it's running Windows with bash and strange paths, so ignore
//       it only for the CI
#[rstest]
#[test(tokio::test)]
#[cfg_attr(all(windows, ci), ignore)]
async fn dir_read_should_support_unlimited_depth_using_zero(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 0,
            /* absolute */ false,
            /* canonicalize */ false,
            /* include_root */ false,
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 4, "Wrong number of entries found");

    assert_eq!(entries[0].file_type, FileType::File);
    assert_eq!(entries[0].path, Path::new("file1"));
    assert_eq!(entries[0].depth, 1);

    assert_eq!(entries[1].file_type, FileType::Symlink);
    assert_eq!(entries[1].path, Path::new("link1"));
    assert_eq!(entries[1].depth, 1);

    assert_eq!(entries[2].file_type, FileType::Dir);
    assert_eq!(entries[2].path, Path::new("sub1"));
    assert_eq!(entries[2].depth, 1);

    assert_eq!(entries[3].file_type, FileType::File);
    assert_eq!(entries[3].path, Path::new("sub1").join("file2"));
    assert_eq!(entries[3].depth, 2);
}

#[rstest]
#[test(tokio::test)]
async fn dir_read_should_support_including_directory_in_returned_entries(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;

    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 1,
            /* absolute */ false,
            /* canonicalize */ false,
            /* include_root */ true,
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 4, "Wrong number of entries found");

    // NOTE: Root entry is always absolute, resolved path
    assert_eq!(entries[0].file_type, FileType::Dir);
    assert_eq!(
        entries[0].path,
        dunce::canonicalize(root_dir.path()).unwrap()
    );
    assert_eq!(entries[0].depth, 0);

    assert_eq!(entries[1].file_type, FileType::File);
    assert_eq!(entries[1].path, Path::new("file1"));
    assert_eq!(entries[1].depth, 1);

    assert_eq!(entries[2].file_type, FileType::Symlink);
    assert_eq!(entries[2].path, Path::new("link1"));
    assert_eq!(entries[2].depth, 1);

    assert_eq!(entries[3].file_type, FileType::Dir);
    assert_eq!(entries[3].path, Path::new("sub1"));
    assert_eq!(entries[3].depth, 1);
}

#[rstest]
#[test(tokio::test)]
async fn dir_read_should_support_returning_absolute_paths(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 1,
            /* absolute */ true,
            /* canonicalize */ false,
            /* include_root */ false,
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 3, "Wrong number of entries found");
    let root_path = dunce::canonicalize(root_dir.path()).unwrap();

    assert_eq!(entries[0].file_type, FileType::File);
    assert_eq!(entries[0].path, root_path.join("file1"));
    assert_eq!(entries[0].depth, 1);

    assert_eq!(entries[1].file_type, FileType::Symlink);
    assert_eq!(entries[1].path, root_path.join("link1"));
    assert_eq!(entries[1].depth, 1);

    assert_eq!(entries[2].file_type, FileType::Dir);
    assert_eq!(entries[2].path, root_path.join("sub1"));
    assert_eq!(entries[2].depth, 1);
}

#[rstest]
#[test(tokio::test)]
async fn dir_read_should_support_returning_canonicalized_paths(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 1,
            /* absolute */ false,
            /* canonicalize */ true,
            /* include_root */ false,
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 3, "Wrong number of entries found");
    println!("{:?}", entries);

    assert_eq!(entries[0].file_type, FileType::File);
    assert_eq!(entries[0].path, Path::new("file1"));
    assert_eq!(entries[0].depth, 1);

    assert_eq!(entries[1].file_type, FileType::Dir);
    assert_eq!(entries[1].path, Path::new("sub1"));
    assert_eq!(entries[1].depth, 1);

    // Symlink should be resolved from $ROOT/link1 -> $ROOT/sub1/file2
    assert_eq!(entries[2].file_type, FileType::Symlink);
    assert_eq!(entries[2].path, Path::new("sub1").join("file2"));
    assert_eq!(entries[2].depth, 1);
}

#[rstest]
#[test(tokio::test)]
async fn create_dir_should_send_error_if_fails(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // Make a path that has multiple non-existent components
    // so the creation will fail
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("nested").join("new-dir");

    let _ = client
        .create_dir(path.to_path_buf(), /* all */ false)
        .await
        .unwrap_err();

    // Also verify that the directory was not actually created
    assert!(!path.exists(), "Path unexpectedly exists");
}

#[rstest]
#[test(tokio::test)]
async fn create_dir_should_send_ok_when_successful(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("new-dir");

    client
        .create_dir(path.to_path_buf(), /* all */ false)
        .await
        .unwrap();

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}

#[rstest]
#[test(tokio::test)]
async fn create_dir_should_support_creating_multiple_dir_components(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("nested").join("new-dir");

    client
        .create_dir(path.to_path_buf(), /* all */ true)
        .await
        .unwrap();

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_send_error_on_failure(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    let _ = client
        .remove(file.path().to_path_buf(), /* false */ false)
        .await
        .unwrap_err();

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_support_deleting_a_directory(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    client
        .remove(dir.path().to_path_buf(), /* false */ false)
        .await
        .unwrap();

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_delete_nonempty_directory_if_force_is_true(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    client
        .remove(dir.path().to_path_buf(), /* false */ true)
        .await
        .unwrap();

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_delete_deeply_nested_directory_if_force_is_true(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let subdir = dir.child("subdir");
    subdir.create_dir_all().unwrap();
    subdir.child("file").touch().unwrap();
    let subdir2 = subdir.child("subsubdir");
    subdir2.create_dir_all().unwrap();
    subdir2.child("file2").touch().unwrap();

    client
        .remove(dir.path().to_path_buf(), /* force */ true)
        .await
        .unwrap();

    dir.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn copy_should_send_error_on_failure(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");

    let _ = client
        .copy(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap_err();

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn copy_should_support_copying_an_entire_directory(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str("some contents").unwrap();

    let dst = temp.child("dst");
    let dst_file = dst.child("file");

    client
        .copy(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir());
    src_file.assert(predicate::path::is_file());
    dst.assert(predicate::path::is_dir());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
#[test(tokio::test)]
async fn copy_should_support_copying_an_empty_directory(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let dst = temp.child("dst");

    client
        .copy(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we still have source and destination directories
    src.assert(predicate::path::is_dir());
    dst.assert(predicate::path::is_dir());
}

#[rstest]
#[test(tokio::test)]
async fn copy_should_support_copying_a_directory_that_only_contains_directories(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_dir = src.child("dir");
    src_dir.create_dir_all().unwrap();

    let dst = temp.child("dst");
    let dst_dir = dst.child("dir");

    client
        .copy(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir().name("src"));
    src_dir.assert(predicate::path::is_dir().name("src/dir"));
    dst.assert(predicate::path::is_dir().name("dst"));
    dst_dir.assert(predicate::path::is_dir().name("dst/dir"));
}

#[rstest]
#[test(tokio::test)]
async fn rename_should_fail_if_path_missing(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");

    let _ = client
        .rename(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap_err();

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn rename_should_support_renaming_an_entire_directory(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str("some contents").unwrap();

    let dst = temp.child("dst");
    let dst_file = dst.child("file");

    client
        .rename(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we moved the contents
    src.assert(predicate::path::missing());
    src_file.assert(predicate::path::missing());
    dst.assert(predicate::path::is_dir());
    dst_file.assert("some contents");
}

#[rstest]
#[test(tokio::test)]
async fn rename_should_support_renaming_a_single_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    client
        .rename(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we moved the file
    src.assert(predicate::path::missing());
    dst.assert("some text");
}

#[rstest]
#[test(tokio::test)]
async fn watch_should_fail_as_unsupported(#[future] client: Ctx<Client>) {
    // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let err = client
        .watch(
            file.path().to_path_buf(),
            /* recursive */ false,
            /* only */ ChangeKindSet::default(),
            /* except */ ChangeKindSet::default(),
        )
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::Unsupported, "{:?}", err);
}

#[rstest]
#[test(tokio::test)]
async fn exists_should_send_true_if_path_exists(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    let exists = client.exists(file.path().to_path_buf()).await.unwrap();
    assert!(exists, "Expected exists to be true, but was false");
}

#[rstest]
#[test(tokio::test)]
async fn exists_should_send_false_if_path_does_not_exist(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");

    let exists = client.exists(file.path().to_path_buf()).await.unwrap();
    assert!(!exists, "Expected exists to be false, but was true");
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_send_error_on_failure(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");

    let _ = client
        .metadata(
            file.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap_err();
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_send_back_metadata_on_file_if_exists(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let metadata = client
        .metadata(
            file.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    assert!(
        matches!(
            metadata,
            Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 9,
                readonly: false,
                ..
            }
        ),
        "{:?}",
        metadata
    );
}

#[cfg(unix)]
#[rstest]
#[test(tokio::test)]
async fn metadata_should_include_unix_specific_metadata_on_unix_platform(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let metadata = client
        .metadata(
            file.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    #[allow(clippy::match_single_binding)]
    match metadata {
        Metadata { unix, windows, .. } => {
            assert!(unix.is_some(), "Unexpectedly missing unix metadata on unix");
            assert!(
                windows.is_none(),
                "Unexpectedly got windows metadata on unix"
            );
        }
    }
}

#[cfg(windows)]
#[rstest]
#[test(tokio::test)]
async fn metadata_should_not_include_windows_as_ssh_cannot_retrieve_that_information(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let metadata = client
        .metadata(
            file.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    #[allow(clippy::match_single_binding)]
    match metadata {
        Metadata { unix, windows, .. } => {
            assert!(
                windows.is_none(),
                "Unexpectedly got windows metadata on windows (support added?)"
            );

            // NOTE: Still includes unix metadata
            assert!(
                unix.is_some(),
                "Unexpectedly missing unix metadata from sshd (even on windows)"
            );
        }
    }
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_send_back_metadata_on_dir_if_exists(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let metadata = client
        .metadata(
            dir.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    assert!(
        matches!(
            metadata,
            Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                readonly: false,
                ..
            }
        ),
        "{:?}",
        metadata
    );
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_send_back_metadata_on_symlink_if_exists(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let metadata = client
        .metadata(
            symlink.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    assert!(
        matches!(
            metadata,
            Metadata {
                canonicalized_path: None,
                file_type: FileType::Symlink,
                readonly: false,
                ..
            }
        ),
        "{:?}",
        metadata
    );
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_include_canonicalized_path_if_flag_specified(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let metadata = client
        .metadata(
            symlink.path().to_path_buf(),
            /* canonicalize */ true,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    // NOTE: This is failing on windows as the symlink does not get resolved!
    match metadata {
        Metadata {
            canonicalized_path: Some(path),
            file_type: FileType::Symlink,
            readonly: false,
            ..
        } => assert_eq!(
            path,
            dunce::canonicalize(file.path()).unwrap(),
            "Symlink canonicalized path does not match referenced file"
        ),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_resolve_file_type_of_symlink_if_flag_specified(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let metadata = client
        .metadata(
            symlink.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ true,
        )
        .await
        .unwrap();

    assert!(
        matches!(
            metadata,
            Metadata {
                file_type: FileType::File,
                ..
            }
        ),
        "{:?}",
        metadata
    );
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_set_readonly_flag_if_specified(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    // Verify that not readonly by default
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File is already set to readonly");

    // Change the file permissions
    client
        .set_permissions(
            file.path().to_path_buf(),
            Permissions::readonly(),
            Default::default(),
        )
        .await
        .unwrap();

    // Retrieve permissions to verify set
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "File not set to readonly");
}

#[allow(unused_attributes)]
#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_set_unix_permissions_if_on_unix_platform(
    #[future] client: Ctx<Client>,
) {
    #[allow(unused_mut, unused_variables)]
    let mut client = client.await;

    #[cfg(unix)]
    {
        use std::os::unix::prelude::*;

        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        // Verify that permissions do not match our readonly state
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        let mode = permissions.mode() & 0o777;
        assert_ne!(mode, 0o400, "File is already set to 0o400");

        // Change the file permissions
        client
            .set_permissions(
                file.path().to_path_buf(),
                Permissions::from_unix_mode(0o400),
                Default::default(),
            )
            .await
            .unwrap();

        // Retrieve file permissions to verify set
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();

        // Drop the upper bits that mode can have (only care about read/write/exec)
        let mode = permissions.mode() & 0o777;

        assert_eq!(mode, 0o400, "Wrong permissions on file: {:o}", mode);
    }
    #[cfg(not(unix))]
    {
        unreachable!();
    }
}

#[allow(unused_attributes)]
#[rstest]
#[test(tokio::test)]
#[cfg_attr(unix, ignore)]
async fn set_permissions_should_set_readonly_flag_if_not_on_unix_platform(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    // Verify that not readonly by default
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File is already set to readonly");

    // Change the file permissions to be readonly (in general)
    client
        .set_permissions(
            file.path().to_path_buf(),
            Permissions::from_unix_mode(0o400),
            Default::default(),
        )
        .await
        .unwrap();

    #[cfg(not(unix))]
    {
        // Retrieve file permissions to verify set
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();

        assert!(permissions.readonly(), "File not marked as readonly");
    }
    #[cfg(unix)]
    {
        unreachable!();
    }
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_not_recurse_if_option_false(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    // Verify that dir is not readonly by default
    let permissions = tokio::fs::symlink_metadata(temp.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Temp dir is already set to readonly"
    );

    // Verify that file is not readonly by default
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File is already set to readonly");

    // Verify that symlink is not readonly by default
    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink is already set to readonly"
    );

    // Change the permissions of the directory and not the contents underneath
    client
        .set_permissions(
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                recursive: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions of the file, symlink, and directory to verify set
    let permissions = tokio::fs::symlink_metadata(temp.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "Temp directory not set to readonly");

    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File unexpectedly set to readonly");

    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink unexpectedly set to readonly"
    );
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_traverse_symlinks_while_recursing_if_following_symlinks_enabled(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let temp2 = assert_fs::TempDir::new().unwrap();
    let file2 = temp2.child("file");
    file2.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_dir(temp2.path()).unwrap();

    // Verify that symlink is not readonly by default
    let permissions = tokio::fs::symlink_metadata(file2.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File2 is already set to readonly");

    // Change the main directory permissions
    client
        .set_permissions(
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: true,
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions referenced by another directory
    let permissions = tokio::fs::symlink_metadata(file2.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "File2 not set to readonly");
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_not_traverse_symlinks_while_recursing_if_following_symlinks_disabled(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let temp2 = assert_fs::TempDir::new().unwrap();
    let file2 = temp2.child("file");
    file2.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_dir(temp2.path()).unwrap();

    // Verify that symlink is not readonly by default
    let permissions = tokio::fs::symlink_metadata(file2.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File2 is already set to readonly");

    // Change the main directory permissions
    client
        .set_permissions(
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: false,
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions referenced by another directory
    let permissions = tokio::fs::symlink_metadata(file2.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "File2 unexpectedly set to readonly"
    );
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_skip_symlinks_if_exclude_symlinks_enabled(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    // Verify that symlink is not readonly by default
    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink is already set to readonly"
    );

    // Change the symlink permissions
    client
        .set_permissions(
            symlink.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                exclude_symlinks: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions to verify not set
    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink (or file underneath) set to readonly"
    );
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_support_recursive_if_option_specified(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    // Verify that dir is not readonly by default
    let permissions = tokio::fs::symlink_metadata(temp.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Temp dir is already set to readonly"
    );

    // Verify that file is not readonly by default
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File is already set to readonly");

    // Change the permissions of the file pointed to by the symlink
    client
        .set_permissions(
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions of the file, symlink, and directory to verify set
    let permissions = tokio::fs::symlink_metadata(temp.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "Temp directory not set to readonly");

    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "File not set to readonly");
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(not(unix), ignore)]
async fn set_permissions_should_support_following_symlinks_if_option_specified(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    // Verify that file is not readonly by default
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(!permissions.readonly(), "File is already set to readonly");

    // Verify that symlink is not readonly by default
    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink is already set to readonly"
    );

    // Change the permissions of the file pointed to by the symlink
    client
        .set_permissions(
            symlink.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Retrieve permissions of the file and symlink to verify set
    let permissions = tokio::fs::symlink_metadata(file.path())
        .await
        .unwrap()
        .permissions();
    assert!(permissions.readonly(), "File not set to readonly");

    let permissions = tokio::fs::symlink_metadata(symlink.path())
        .await
        .unwrap()
        .permissions();
    assert!(
        !permissions.readonly(),
        "Symlink unexpectedly set to readonly"
    );
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_not_fail_even_if_process_not_found(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // NOTE: This is a distinction from standard distant and ssh distant
    let _ = client
        .spawn(
            /* cmd */ DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_return_id_of_spawned_process(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("echo hello", "echo hello");
    let proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();
    assert!(proc.id() > 0);
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_send_back_stdout_periodically_when_available(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;

    let cmd = platform_cmd("sh -c 'printf \"%s\" \"some stdout\"'", "echo some stdout");
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    let stdout_pipe = proc.stdout.as_mut().unwrap();
    let mut accumulated = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, stdout_pipe.read()).await {
            Ok(Ok(data)) => {
                accumulated.extend_from_slice(&data);
                let s = String::from_utf8_lossy(&accumulated);
                if s.contains("some stdout") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    let stdout_str = String::from_utf8_lossy(&accumulated);
    assert!(
        stdout_str.contains("some stdout"),
        "Expected 'some stdout', got '{}'",
        stdout_str.trim()
    );
    assert!(
        proc.wait().await.unwrap().success,
        "Process should have completed successfully"
    );
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_send_back_stderr_periodically_when_available(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;

    let cmd = platform_cmd(
        "sh -c 'printf \"%s\" \"some stderr\" >&2'",
        "echo some stderr 1>&2",
    );
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    let stderr_pipe = proc.stderr.as_mut().unwrap();
    let mut accumulated = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, stderr_pipe.read()).await {
            Ok(Ok(data)) => {
                accumulated.extend_from_slice(&data);
                let s = String::from_utf8_lossy(&accumulated);
                if s.contains("some stderr") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    let stderr_str = String::from_utf8_lossy(&accumulated);
    assert!(
        stderr_str.contains("some stderr"),
        "Expected 'some stderr', got '{}'",
        stderr_str.trim()
    );
    assert!(
        proc.wait().await.unwrap().success,
        "Process should have completed successfully"
    );
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_send_done_signal_when_completed(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 0.1", "timeout /t 1 /nobreak >nul");
    let proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    let _ = proc.wait().await.unwrap();
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_clear_process_from_state_when_killed(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 1", "timeout /t 1 /nobreak >nul");
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    // Send kill signal
    proc.kill().await.unwrap();

    // Verify killed, which should be success false
    let status = proc.wait().await.unwrap();
    assert!(!status.success, "Process succeeded when killed")
}

#[rstest]
#[test(tokio::test)]
async fn proc_kill_should_fail_if_process_not_running(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 1", "timeout /t 1 /nobreak >nul");
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    // Send kill signal
    proc.kill().await.unwrap();

    // Wait for process to be dead
    let mut killer = proc.clone_killer();
    let _ = proc.wait().await.unwrap();

    // Now send it again, which should fail
    let _ = killer.kill().await.unwrap_err();
}

#[rstest]
#[test(tokio::test)]
async fn proc_stdin_should_fail_if_process_not_running(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 1", "timeout /t 1 /nobreak >nul");
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    // Send kill signal
    proc.kill().await.unwrap();

    // Wait for process to be dead
    let mut stdin = proc.stdin.take().unwrap();
    let _ = proc.wait().await.unwrap();

    // Now send stdin, which should fail
    let _ = stdin.write_str("some data").await.unwrap_err();
}

#[rstest]
#[test(tokio::test)]
async fn proc_stdin_should_send_stdin_to_process(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    // On Unix, use a simple read-echo loop; on Windows, use PowerShell to read one line from stdin
    // (findstr buffers output when not connected to a console, causing hangs)
    let cmd = platform_cmd(
        "sh -c 'while IFS= read line; do echo \"$line\"; done'",
        "powershell -NonInteractive -Command [Console]::In.ReadLine()",
    );

    // First, run a program that listens for stdin
    let mut proc = client
        .spawn(
            cmd,
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* pty */ None,
        )
        .await
        .unwrap();

    // Second, send stdin to the remote process
    proc.stdin
        .as_mut()
        .unwrap()
        .write_str("hello world\n")
        .await
        .unwrap();

    // Third, check the async response of stdout to verify we got stdin
    let stdout_pipe = proc.stdout.as_mut().unwrap();
    let mut accumulated = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, stdout_pipe.read()).await {
            Ok(Ok(data)) => {
                accumulated.extend_from_slice(&data);
                let s = String::from_utf8_lossy(&accumulated);
                if s.contains("hello world") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    let stdout_str = String::from_utf8_lossy(&accumulated);
    assert!(
        stdout_str.contains("hello world"),
        "Expected 'hello world', got '{}'",
        stdout_str.trim()
    );
}

#[rstest]
#[test(tokio::test)]
async fn system_info_should_return_system_info_based_on_binary(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let system_info = client.system_info().await.unwrap();

    assert_eq!(system_info.family, std::env::consts::FAMILY.to_string());

    // We only support setting the os when the family is windows
    if system_info.family == "windows" {
        assert_eq!(system_info.os, "windows");
    } else {
        assert_eq!(system_info.os, "");
    }

    assert_eq!(system_info.arch, "");
    assert_eq!(system_info.main_separator, std::path::MAIN_SEPARATOR);

    // We don't have an easy way to tell the remote username and shell in most cases,
    // so we just check that they are not empty
    assert_ne!(system_info.username, "");
    assert_ne!(system_info.shell, "");
}

#[rstest]
#[test(tokio::test)]
async fn version_should_return_server_version_and_capabilities(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let version = client.version().await.unwrap();

    // Verify server version is parseable and contains our crate name as build metadata
    assert!(
        version
            .server_version
            .build
            .as_str()
            .contains("distant-ssh"),
        "Server version build metadata should contain 'distant-ssh', got: {}",
        version.server_version
    );

    // Verify capabilities include expected ones
    let caps = &version.capabilities;
    assert!(
        caps.iter().any(|c| c.contains("exec")),
        "Missing exec capability: {:?}",
        caps
    );
    assert!(
        caps.iter().any(|c| c.contains("fs_io")),
        "Missing fs_io capability: {:?}",
        caps
    );
    assert!(
        caps.iter().any(|c| c.contains("sys_info")),
        "Missing sys_info capability: {:?}",
        caps
    );
}

#[rstest]
#[test(tokio::test)]
async fn search_should_fail_as_unsupported(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();

    let result = client
        .search(SearchQuery {
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Contains {
                value: "test".to_string(),
            },
            paths: vec![temp.path().to_path_buf()],
            options: Default::default(),
        })
        .await;

    assert!(result.is_err(), "Search should fail as unsupported");
}

#[rstest]
#[test(tokio::test)]
async fn cancel_search_should_fail_as_unsupported(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let result = client.cancel_search(0).await;

    assert!(result.is_err(), "Cancel search should fail as unsupported");
}

#[rstest]
#[test(tokio::test)]
async fn unwatch_should_fail_as_unsupported(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    let result = client.unwatch(file.path().to_path_buf()).await;

    assert!(result.is_err(), "Unwatch should fail as unsupported");
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_with_pty_should_return_id(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("echo hello", "echo hello");
    let proc = client
        .spawn(
            cmd,
            Environment::new(),
            None,
            Some(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )
        .await
        .unwrap();

    assert!(proc.id() > 0);
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_with_pty_should_be_killable(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 30", "timeout /t 30 /nobreak >nul");
    let mut proc = client
        .spawn(
            cmd,
            Environment::new(),
            None,
            Some(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )
        .await
        .unwrap();

    // Kill the process
    proc.kill().await.unwrap();

    // Verify killed
    let status = proc.wait().await.unwrap();
    assert!(!status.success, "PTY process succeeded when killed");
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_with_pty_should_support_resize(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let cmd = platform_cmd("sleep 30", "timeout /t 30 /nobreak >nul");
    let mut proc = client
        .spawn(
            cmd,
            Environment::new(),
            None,
            Some(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )
        .await
        .unwrap();

    // Resize should succeed without error
    proc.resize(PtySize {
        rows: 48,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    })
    .await
    .unwrap();

    // Clean up
    proc.kill().await.unwrap();
    let _ = proc.wait().await.unwrap();
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_fail_if_current_dir_specified(#[future] client: Ctx<Client>) {
    let mut client = client.await;

    let current_dir = if cfg!(windows) { "C:\\temp" } else { "/tmp" };
    let result = client
        .spawn(
            "echo hello".to_string(),
            Environment::new(),
            Some(std::path::PathBuf::from(current_dir)),
            None,
        )
        .await;
    assert!(result.is_err());
}

#[rstest]
#[test(tokio::test)]
#[cfg_attr(all(windows, ci), ignore)]
async fn dir_read_should_support_explicit_depth_greater_than_one(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let root_dir = assert_fs::TempDir::new().unwrap();
    root_dir.child("file1").touch().unwrap();
    let sub1 = root_dir.child("sub1");
    sub1.create_dir_all().unwrap();
    sub1.child("file2").touch().unwrap();
    let sub2 = sub1.child("sub2");
    sub2.create_dir_all().unwrap();
    sub2.child("file3").touch().unwrap();

    let (entries, _) = client
        .read_dir(
            root_dir.path().to_path_buf(),
            /* depth */ 2,
            /* absolute */ false,
            /* canonicalize */ false,
            /* include_root */ false,
        )
        .await
        .unwrap();

    // depth=2 should include root-level + sub1 contents, but NOT sub2/file3
    let paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("file1")),
        "Missing file1: {:?}",
        paths
    );
    assert!(
        paths.iter().any(|p| p.ends_with("sub1")),
        "Missing sub1: {:?}",
        paths
    );
    assert!(
        paths.iter().any(|p| p.ends_with("file2")),
        "Missing file2 at depth 2: {:?}",
        paths
    );
    // sub2 directory should appear at depth 2
    assert!(
        paths.iter().any(|p| p.ends_with("sub2")),
        "Missing sub2 at depth 2: {:?}",
        paths
    );
    // But file3 inside sub2 should NOT appear (that would be depth 3)
    assert!(
        !paths.iter().any(|p| p.ends_with("file3")),
        "file3 should NOT appear at depth 2: {:?}",
        paths
    );
}

#[rstest]
#[test(tokio::test)]
async fn exists_should_send_false_if_parent_directory_does_not_exist(
    #[future] client: Ctx<Client>,
) {
    let mut client = client.await;
    let path = std::path::PathBuf::from("/nonexistent_parent_abc123/nonexistent_child");
    let exists = client.exists(path).await.unwrap();
    assert!(
        !exists,
        "Expected exists to be false for deeply nonexistent path"
    );
}

#[rstest]
#[test(tokio::test)]
async fn remove_should_support_deleting_a_single_file(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file-to-remove");
    file.write_str("some content").unwrap();

    client
        .remove(file.path().to_path_buf(), /* force */ false)
        .await
        .unwrap();

    file.assert(predicate::path::missing());
}

#[rstest]
#[test(tokio::test)]
async fn metadata_should_include_modified_timestamp(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("timestamped-file");
    file.write_str("content").unwrap();

    let metadata = client
        .metadata(
            file.path().to_path_buf(),
            /* canonicalize */ false,
            /* resolve_file_type */ false,
        )
        .await
        .unwrap();

    assert!(
        metadata.modified.is_some(),
        "Expected modified timestamp to be set: {:?}",
        metadata
    );
}

#[rstest]
#[test(tokio::test)]
async fn set_permissions_should_fail_if_path_does_not_exist(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let missing = temp.child("nonexistent");

    let result = client
        .set_permissions(
            missing.path().to_path_buf(),
            Permissions::readonly(),
            Default::default(),
        )
        .await;

    assert!(
        result.is_err(),
        "set_permissions on missing path should fail"
    );
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_with_pty_should_fail_if_current_dir_specified(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let current_dir = if cfg!(windows) { "C:\\temp" } else { "/tmp" };
    let result = client
        .spawn(
            "echo hello".to_string(),
            Environment::new(),
            Some(std::path::PathBuf::from(current_dir)),
            Some(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )
        .await;
    assert!(result.is_err(), "PTY spawn with current_dir should fail");
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_with_pty_should_capture_stdout(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let cmd = platform_cmd("echo pty_test_output", "echo pty_test_output");
    let mut proc = client
        .spawn(
            cmd,
            Environment::new(),
            None,
            Some(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )
        .await
        .unwrap();
    // PTY combines stdout/stderr into stdout — read in a loop since PTY may
    // deliver escape sequences before the actual output, especially on Windows CI.
    let stdout_pipe = proc.stdout.as_mut().unwrap();
    let mut accumulated = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, stdout_pipe.read()).await {
            Ok(Ok(data)) => {
                accumulated.extend_from_slice(&data);
                let s = String::from_utf8_lossy(&accumulated);
                if s.contains("pty_test_output") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    let stdout_str = String::from_utf8_lossy(&accumulated);
    assert!(
        stdout_str.contains("pty_test_output"),
        "Expected stdout to contain 'pty_test_output', got '{}'",
        stdout_str
    );
}

#[rstest]
#[test(tokio::test)]
async fn proc_spawn_should_report_nonzero_exit_code(#[future] client: Ctx<Client>) {
    let mut client = client.await;
    let cmd = platform_cmd("sh -c 'exit 42'", "cmd /C exit 42");
    let proc = client
        .spawn(cmd, Environment::new(), None, None)
        .await
        .unwrap();
    let status = proc.wait().await.unwrap();
    assert!(
        !status.success,
        "Process with exit 42 should not be success"
    );
    assert_eq!(
        status.code,
        Some(42),
        "Expected exit code 42, got {:?}",
        status.code
    );
}
