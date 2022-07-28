use crate::sshd::*;
use assert_fs::{prelude::*, TempDir};
use distant_core::{
    data::{ChangeKindSet, Environment, FileType, Metadata},
    DistantChannelExt, DistantClient,
};
use once_cell::sync::Lazy;
use predicates::prelude::*;
use rstest::*;
use std::{io, path::Path, time::Duration};

static TEMP_SCRIPT_DIR: Lazy<TempDir> = Lazy::new(|| TempDir::new().unwrap());
static SCRIPT_RUNNER: Lazy<String> = Lazy::new(|| String::from("bash"));

static ECHO_ARGS_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                printf "%s" "$*"
            "#
        ))
        .unwrap();
    script
});

static ECHO_ARGS_TO_STDERR_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                printf "%s" "$*" 1>&2
            "#
        ))
        .unwrap();
    script
});

static ECHO_STDIN_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                while IFS= read; do echo "$REPLY"; done
            "#
        ))
        .unwrap();
    script
});

static SLEEP_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("sleep.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #!/usr/bin/env bash
                sleep "$1"
            "#
        ))
        .unwrap();
    script
});

static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

#[rstest]
#[tokio::test]
async fn read_file_should_fail_if_file_missing(#[future] client: DistantClient) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let path = temp.child("missing-file").path().to_path_buf();

    let _ = client.read_file(path).await.unwrap_err();
}

#[rstest]
#[tokio::test]
async fn read_file_should_send_blob_with_file_contents(#[future] client: DistantClient) {
    let mut client = client.await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let bytes = client.read_file(file.path().to_path_buf()).await.unwrap();
    assert_eq!(bytes, b"some file contents");
}

#[rstest]
#[tokio::test]
async fn read_file_text_should_send_error_if_fails_to_read_file(#[future] client: DistantClient) {
    let mut client = client.await;

    let temp = assert_fs::TempDir::new().unwrap();
    let path = temp.child("missing-file").path().to_path_buf();

    let _ = client.read_file_text(path).await.unwrap_err();
}

#[rstest]
#[tokio::test]
async fn read_file_text_should_send_text_with_file_contents(#[future] client: DistantClient) {
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
#[tokio::test]
async fn write_file_should_send_error_if_fails_to_write_file(#[future] client: DistantClient) {
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
#[tokio::test]
async fn write_file_should_send_ok_when_successful(#[future] client: DistantClient) {
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
#[tokio::test]
async fn write_file_text_should_send_error_if_fails_to_write_file(#[future] client: DistantClient) {
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
#[tokio::test]
async fn write_file_text_should_send_ok_when_successful(#[future] client: DistantClient) {
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
#[tokio::test]
async fn append_file_should_send_error_if_fails_to_create_file(#[future] client: DistantClient) {
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
#[tokio::test]
async fn append_file_should_create_file_if_missing(#[future] client: DistantClient) {
    let mut client = client.await;

    // Don't create the file directly, but define path
    // where the file should be
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .append_file(file.path().to_path_buf(), b"some extra contents".to_vec())
        .await
        .unwrap();

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did create to the file
    file.assert("some extra contents");
}

#[rstest]
#[tokio::test]
async fn append_file_should_send_ok_when_successful(#[future] client: DistantClient) {
    let mut client = client.await;

    // Create a temporary file and fill it with some contents
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    client
        .append_file(file.path().to_path_buf(), b"some extra contents".to_vec())
        .await
        .unwrap();

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[tokio::test]
async fn append_file_text_should_send_error_if_fails_to_create_file(
    #[future] client: DistantClient,
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
#[tokio::test]
async fn append_file_text_should_create_file_if_missing(#[future] client: DistantClient) {
    let mut client = client.await;

    // Don't create the file directly, but define path
    // where the file should be
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap();

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did create to the file
    file.assert("some extra contents");
}

#[rstest]
#[tokio::test]
async fn append_file_text_should_send_ok_when_successful(#[future] client: DistantClient) {
    let mut client = client.await;

    // Create a temporary file and fill it with some contents
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    client
        .append_file_text(file.path().to_path_buf(), "some extra contents".to_string())
        .await
        .unwrap();

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[tokio::test]
async fn dir_read_should_send_error_if_directory_does_not_exist(#[future] client: DistantClient) {
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
    root_dir.child("file1").touch().unwrap();

    let sub1 = root_dir.child("sub1");
    sub1.create_dir_all().unwrap();

    let file2 = sub1.child("file2");
    file2.touch().unwrap();

    let link1 = root_dir.child("link1");
    link1.symlink_to_file(file2.path()).unwrap();

    root_dir
}

#[rstest]
#[tokio::test]
async fn dir_read_should_support_depth_limits(#[future] client: DistantClient) {
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

#[rstest]
#[tokio::test]
async fn dir_read_should_support_unlimited_depth_using_zero(#[future] client: DistantClient) {
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
#[tokio::test]
async fn dir_read_should_support_including_directory_in_returned_entries(
    #[future] client: DistantClient,
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
    assert_eq!(entries[0].path, root_dir.path().canonicalize().unwrap());
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
#[tokio::test]
async fn dir_read_should_support_returning_absolute_paths(#[future] client: DistantClient) {
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
    let root_path = root_dir.path().canonicalize().unwrap();

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
#[tokio::test]
async fn dir_read_should_support_returning_canonicalized_paths(#[future] client: DistantClient) {
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
#[tokio::test]
async fn create_dir_should_send_error_if_fails(#[future] client: DistantClient) {
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
#[tokio::test]
async fn create_dir_should_send_ok_when_successful(#[future] client: DistantClient) {
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
#[tokio::test]
async fn create_dir_should_support_creating_multiple_dir_components(
    #[future] client: DistantClient,
) {
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
#[tokio::test]
async fn remove_should_send_error_on_failure(#[future] client: DistantClient) {
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
#[tokio::test]
async fn remove_should_support_deleting_a_directory(#[future] client: DistantClient) {
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
#[tokio::test]
async fn remove_should_delete_nonempty_directory_if_force_is_true(#[future] client: DistantClient) {
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
#[tokio::test]
async fn remove_should_support_deleting_a_single_file(#[future] client: DistantClient) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("some-file");
    file.touch().unwrap();

    client
        .remove(file.path().to_path_buf(), /* false */ false)
        .await
        .unwrap();

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn copy_should_send_error_on_failure(#[future] client: DistantClient) {
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
#[tokio::test]
async fn copy_should_support_copying_an_entire_directory(#[future] client: DistantClient) {
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
#[tokio::test]
async fn copy_should_support_copying_an_empty_directory(#[future] client: DistantClient) {
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
#[tokio::test]
async fn copy_should_support_copying_a_directory_that_only_contains_directories(
    #[future] client: DistantClient,
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
#[tokio::test]
async fn copy_should_support_copying_a_single_file(#[future] client: DistantClient) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    client
        .copy(src.path().to_path_buf(), dst.path().to_path_buf())
        .await
        .unwrap();

    // Verify that we still have source and that destination has source's contents
    src.assert(predicate::path::is_file());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
#[tokio::test]
async fn rename_should_fail_if_path_missing(#[future] client: DistantClient) {
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
#[tokio::test]
async fn rename_should_support_renaming_an_entire_directory(#[future] client: DistantClient) {
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
#[tokio::test]
async fn rename_should_support_renaming_a_single_file(#[future] client: DistantClient) {
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
#[tokio::test]
async fn watch_should_fail_as_unsupported(#[future] client: DistantClient) {
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
#[tokio::test]
async fn exists_should_send_true_if_path_exists(#[future] client: DistantClient) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    let exists = client.exists(file.path().to_path_buf()).await.unwrap();
    assert!(exists, "Expected exists to be true, but was false");
}

#[rstest]
#[tokio::test]
async fn exists_should_send_false_if_path_does_not_exist(#[future] client: DistantClient) {
    let mut client = client.await;
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");

    let exists = client.exists(file.path().to_path_buf()).await.unwrap();
    assert!(!exists, "Expected exists to be false, but was true");
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_error_on_failure(#[future] client: DistantClient) {
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
#[tokio::test]
async fn metadata_should_send_back_metadata_on_file_if_exists(#[future] client: DistantClient) {
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
#[tokio::test]
async fn metadata_should_include_unix_specific_metadata_on_unix_platform(
    #[future] client: DistantClient,
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
#[tokio::test]
async fn metadata_should_not_include_windows_as_ssh_cannot_retrieve_that_information(
    #[future] client: DistantClient,
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
            assert!(unix.is_none(), "Unexpectedly got unix metadata on windows");
        }
    }
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_back_metadata_on_dir_if_exists(#[future] client: DistantClient) {
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
#[tokio::test]
async fn metadata_should_send_back_metadata_on_symlink_if_exists(#[future] client: DistantClient) {
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
#[tokio::test]
async fn metadata_should_include_canonicalized_path_if_flag_specified(
    #[future] client: DistantClient,
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

    match metadata {
        Metadata {
            canonicalized_path: Some(path),
            file_type: FileType::Symlink,
            readonly: false,
            ..
        } => assert_eq!(
            path,
            file.path().canonicalize().unwrap(),
            "Symlink canonicalized path does not match referenced file"
        ),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn metadata_should_resolve_file_type_of_symlink_if_flag_specified(
    #[future] client: DistantClient,
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
#[tokio::test]
async fn proc_spawn_should_not_fail_even_if_process_not_found(#[future] client: DistantClient) {
    let mut client = client.await;

    // NOTE: This is a distinction from standard distant and ssh distant
    let _ = client
        .spawn(
            /* cmd */ DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
            /* pty */ None,
        )
        .await
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn proc_spawn_should_return_id_of_spawned_process(#[future] client: DistantClient) {
    let mut client = client.await;

    let proc = client
        .spawn(
            /* cmd */
            format!(
                "{} {}",
                *SCRIPT_RUNNER,
                ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap()
            ),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
            /* pty */ None,
        )
        .await
        .unwrap();
    assert!(proc.id() > 0);
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_spawn_should_send_back_stdout_periodically_when_available(
    #[future] client: DistantClient,
) {
    let mut client = client.await;

    let mut proc = client
        .spawn(
            /* cmd */
            format!(
                "{} {} some stdout",
                *SCRIPT_RUNNER,
                ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap()
            ),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
            /* pty */ None,
        )
        .await
        .unwrap();

    assert_eq!(
        proc.stdout.as_mut().unwrap().read().await.unwrap(),
        b"some stdout"
    );
    assert!(
        proc.wait().await.unwrap().success,
        "Process should have completed successfully"
    );
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_spawn_should_send_back_stderr_periodically_when_available(
    #[future] client: DistantClient,
) {
    let mut client = client.await;

    let mut proc = client
        .spawn(
            /* cmd */
            format!(
                "{} {} some stderr",
                *SCRIPT_RUNNER,
                ECHO_ARGS_TO_STDERR_SH.to_str().unwrap()
            ),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
            /* pty */ None,
        )
        .await
        .unwrap();

    assert_eq!(
        proc.stderr.as_mut().unwrap().read().await.unwrap(),
        b"some stderr"
    );
    assert!(
        proc.wait().await.unwrap().success,
        "Process should have completed successfully"
    );
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_spawn_should_send_done_signal_when_completed(#[future] client: DistantClient) {
    let mut client = client.await;

    let proc = client
        .spawn(
            /* cmd */
            format!("{} {} 0.1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
            /* pty */ None,
        )
        .await
        .unwrap();

    let _ = proc.wait().await.unwrap();
}

#[rstest]
#[tokio::test]
async fn proc_spawn_should_clear_process_from_state_when_killed(#[future] client: DistantClient) {
    let mut client = client.await;

    let mut proc = client
        .spawn(
            /* cmd */
            format!("{} {} 1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
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
#[tokio::test]
async fn proc_kill_should_fail_if_process_not_running(#[future] client: DistantClient) {
    let mut client = client.await;

    let mut proc = client
        .spawn(
            /* cmd */
            format!("{} {} 1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
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
#[tokio::test]
async fn proc_stdin_should_fail_if_process_not_running(#[future] client: DistantClient) {
    let mut client = client.await;

    let mut proc = client
        .spawn(
            /* cmd */
            format!("{} {} 1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
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

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_stdin_should_send_stdin_to_process(#[future] client: DistantClient) {
    let mut client = client.await;

    // First, run a program that listens for stdin
    let mut proc = client
        .spawn(
            /* cmd */
            format!(
                "{} {}",
                *SCRIPT_RUNNER,
                ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap()
            ),
            /* environment */ Environment::new(),
            /* current_dir */ None,
            /* persist */ false,
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
    assert_eq!(
        proc.stdout.as_mut().unwrap().read_string().await.unwrap(),
        "hello world\n"
    );
}

#[rstest]
#[tokio::test]
async fn system_info_should_return_system_info_based_on_binary(#[future] client: DistantClient) {
    let mut client = client.await;

    let system_info = client.system_info().await.unwrap();
    assert_eq!(system_info.family, std::env::consts::FAMILY.to_string());
    assert_eq!(system_info.os, "");
    assert_eq!(system_info.arch, "");
    assert_eq!(system_info.main_separator, std::path::MAIN_SEPARATOR);
}
