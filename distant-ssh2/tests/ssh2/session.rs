use crate::sshd::*;
use assert_fs::{prelude::*, TempDir};
use distant_core::{
    FileType, Request, RequestData, Response, ResponseData, RunningProcess, Session,
};
use predicates::prelude::*;
use rstest::*;
use std::{env, path::Path, time::Duration};

lazy_static::lazy_static! {
    static ref TEMP_SCRIPT_DIR: TempDir = TempDir::new().unwrap();
    static ref SCRIPT_RUNNER: String = String::from("bash");

    static ref ECHO_ARGS_TO_STDOUT_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
        script.write_str(indoc::indoc!(r#"
                #/usr/bin/env bash
                printf "%s" "$@"
            "#)).unwrap();
        script
    };

    static ref ECHO_ARGS_TO_STDERR_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
        script.write_str(indoc::indoc!(r#"
                #/usr/bin/env bash
                printf "%s" "$@" 1>&2
            "#)).unwrap();
        script
    };

    static ref ECHO_STDIN_TO_STDOUT_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
        script.write_str(indoc::indoc!(r#"
                #/usr/bin/env bash
                while IFS= read; do echo "$REPLY"; done
            "#)).unwrap();
        script
    };

    static ref EXIT_CODE_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("exit_code.sh");
        script.write_str(indoc::indoc!(r#"
                #!/usr/bin/env bash
                exit "$1"
            "#)).unwrap();
        script
    };

    static ref SLEEP_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("sleep.sh");
        script.write_str(indoc::indoc!(r#"
                #!/usr/bin/env bash
                sleep "$1"
            "#)).unwrap();
        script
    };

    static ref DOES_NOT_EXIST_BIN: assert_fs::fixture::ChildPath =
        TEMP_SCRIPT_DIR.child("does_not_exist_bin");
}

#[rstest]
#[tokio::test]
async fn file_read_should_send_error_if_fails_to_read_file(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let path = temp.child("missing-file").path().to_path_buf();
    let req = Request::new("test-tenant", vec![RequestData::FileRead { path }]);
    let res = session.send(req).await.unwrap();

    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn file_read_should_send_blob_with_file_contents(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileRead {
            path: file.path().to_path_buf(),
        }],
    );
    let res = session.send(req).await.unwrap();

    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::Blob { data } => assert_eq!(data, b"some file contents"),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn file_read_text_should_send_error_if_fails_to_read_file(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let path = temp.child("missing-file").path().to_path_buf();
    let req = Request::new("test-tenant", vec![RequestData::FileReadText { path }]);
    let res = session.send(req).await.unwrap();

    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn file_read_text_should_send_text_with_file_contents(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileReadText {
            path: file.path().to_path_buf(),
        }],
    );
    let res = session.send(req).await.unwrap();

    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::Text { data } => assert_eq!(data, "some file contents"),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn file_write_should_send_error_if_fails_to_write_file(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileWrite {
            path: file.path().to_path_buf(),
            data: b"some text".to_vec(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn file_write_should_send_ok_when_successful(#[future] session: Session) {
    let mut session = session.await;
    // Path should point to a file that does not exist, but all
    // other components leading up to it do
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileWrite {
            path: file.path().to_path_buf(),
            data: b"some text".to_vec(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we actually did create the file
    // with the associated contents
    file.assert("some text");
}

#[rstest]
#[tokio::test]
async fn file_write_text_should_send_error_if_fails_to_write_file(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileWriteText {
            path: file.path().to_path_buf(),
            text: String::from("some text"),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn file_write_text_should_send_ok_when_successful(#[future] session: Session) {
    let mut session = session.await;
    // Path should point to a file that does not exist, but all
    // other components leading up to it do
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileWriteText {
            path: file.path().to_path_buf(),
            text: String::from("some text"),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we actually did create the file
    // with the associated contents
    file.assert("some text");
}

#[rstest]
#[tokio::test]
async fn file_append_should_send_error_if_fails_to_create_file(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileAppend {
            path: file.path().to_path_buf(),
            data: b"some extra contents".to_vec(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn file_append_should_send_ok_when_successful(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary file and fill it with some contents
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileAppend {
            path: file.path().to_path_buf(),
            data: b"some extra contents".to_vec(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[tokio::test]
async fn file_append_text_should_send_error_if_fails_to_create_file(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileAppendText {
            path: file.path().to_path_buf(),
            text: String::from("some extra contents"),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn file_append_text_should_send_ok_when_successful(#[future] session: Session) {
    let mut session = session.await;
    // Create a temporary file and fill it with some contents
    let temp = TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::FileAppendText {
            path: file.path().to_path_buf(),
            text: String::from("some extra contents"),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Yield to allow chance to finish appending to file
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Also verify that we actually did append to the file
    file.assert("some file contentssome extra contents");
}

#[rstest]
#[tokio::test]
async fn dir_read_should_send_error_if_directory_does_not_exist(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let dir = temp.child("test-dir");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: dir.path().to_path_buf(),
            depth: 0,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

// /root/
// /root/file1
// /root/link1 -> /root/sub1/file2
// /root/sub1/
// /root/sub1/file2
async fn setup_dir() -> TempDir {
    let root_dir = TempDir::new().unwrap();
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
async fn dir_read_should_support_depth_limits(#[future] session: Session) {
    let mut session = session.await;
    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: root_dir.path().to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::DirEntries { entries, .. } => {
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
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn dir_read_should_support_unlimited_depth_using_zero(#[future] session: Session) {
    let mut session = session.await;
    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: root_dir.path().to_path_buf(),
            depth: 0,
            absolute: false,
            canonicalize: false,
            include_root: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::DirEntries { entries, .. } => {
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
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn dir_read_should_support_including_directory_in_returned_entries(
    #[future] session: Session,
) {
    let mut session = session.await;
    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: root_dir.path().to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: false,
            include_root: true,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::DirEntries { entries, .. } => {
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
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn dir_read_should_support_returning_absolute_paths(#[future] session: Session) {
    let mut session = session.await;
    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: root_dir.path().to_path_buf(),
            depth: 1,
            absolute: true,
            canonicalize: false,
            include_root: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::DirEntries { entries, .. } => {
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
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn dir_read_should_support_returning_canonicalized_paths(#[future] session: Session) {
    let mut session = session.await;
    // Create directory with some nested items
    let root_dir = setup_dir().await;

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirRead {
            path: root_dir.path().to_path_buf(),
            depth: 1,
            absolute: false,
            canonicalize: true,
            include_root: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::DirEntries { entries, .. } => {
            assert_eq!(entries.len(), 3, "Wrong number of entries found");

            assert_eq!(entries[0].file_type, FileType::File);
            assert_eq!(entries[0].path, Path::new("file1"));
            assert_eq!(entries[0].depth, 1);

            // Symlink should be resolved from $ROOT/link1 -> $ROOT/sub1/file2
            assert_eq!(entries[1].file_type, FileType::Symlink);
            assert_eq!(entries[1].path, Path::new("sub1").join("file2"));
            assert_eq!(entries[1].depth, 1);

            assert_eq!(entries[2].file_type, FileType::Dir);
            assert_eq!(entries[2].path, Path::new("sub1"));
            assert_eq!(entries[2].depth, 1);
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn dir_create_should_send_error_if_fails(#[future] session: Session) {
    let mut session = session.await;
    // Make a path that has multiple non-existent components
    // so the creation will fail
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("nested").join("new-dir");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirCreate {
            path: path.to_path_buf(),
            all: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that the directory was not actually created
    assert!(!path.exists(), "Path unexpectedly exists");
}

#[rstest]
#[tokio::test]
async fn dir_create_should_send_ok_when_successful(#[future] session: Session) {
    let mut session = session.await;
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("new-dir");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirCreate {
            path: path.to_path_buf(),
            all: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}

#[rstest]
#[tokio::test]
async fn dir_create_should_support_creating_multiple_dir_components(#[future] session: Session) {
    let mut session = session.await;
    let root_dir = setup_dir().await;
    let path = root_dir.path().join("nested").join("new-dir");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::DirCreate {
            path: path.to_path_buf(),
            all: true,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}

#[rstest]
#[tokio::test]
async fn remove_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("missing-file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Remove {
            path: file.path().to_path_buf(),
            force: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn remove_should_support_deleting_a_directory(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Remove {
            path: dir.path().to_path_buf(),
            force: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn remove_should_delete_nonempty_directory_if_force_is_true(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Remove {
            path: dir.path().to_path_buf(),
            force: true,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn remove_should_support_deleting_a_single_file(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("some-file");
    file.touch().unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Remove {
            path: file.path().to_path_buf(),
            force: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn copy_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Copy {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn copy_should_support_copying_an_entire_directory(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str("some contents").unwrap();

    let dst = temp.child("dst");
    let dst_file = dst.child("file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Copy {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir());
    src_file.assert(predicate::path::is_file());
    dst.assert(predicate::path::is_dir());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
#[tokio::test]
async fn copy_should_support_copying_an_empty_directory(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let dst = temp.child("dst");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Copy {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we still have source and destination directories
    src.assert(predicate::path::is_dir());
    dst.assert(predicate::path::is_dir());
}

#[rstest]
#[tokio::test]
async fn copy_should_support_copying_a_directory_that_only_contains_directories(
    #[future] session: Session,
) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_dir = src.child("dir");
    src_dir.create_dir_all().unwrap();

    let dst = temp.child("dst");
    let dst_dir = dst.child("dir");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Copy {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir().name("src"));
    src_dir.assert(predicate::path::is_dir().name("src/dir"));
    dst.assert(predicate::path::is_dir().name("dst"));
    dst_dir.assert(predicate::path::is_dir().name("dst/dir"));
}

#[rstest]
#[tokio::test]
async fn copy_should_support_copying_a_single_file(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Copy {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we still have source and that destination has source's contents
    src.assert(predicate::path::is_file());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
#[tokio::test]
async fn rename_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Rename {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn rename_should_support_renaming_an_entire_directory(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str("some contents").unwrap();

    let dst = temp.child("dst");
    let dst_file = dst.child("file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Rename {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we moved the contents
    src.assert(predicate::path::missing());
    src_file.assert(predicate::path::missing());
    dst.assert(predicate::path::is_dir());
    dst_file.assert("some contents");
}

#[rstest]
#[tokio::test]
async fn rename_should_support_renaming_a_single_file(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Rename {
            src: src.path().to_path_buf(),
            dst: dst.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Ok),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Verify that we moved the file
    src.assert(predicate::path::missing());
    dst.assert("some text");
}

#[rstest]
#[tokio::test]
async fn exists_should_send_true_if_path_exists(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Exists {
            path: file.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert_eq!(res.payload[0], ResponseData::Exists(true));
}

#[rstest]
#[tokio::test]
async fn exists_should_send_false_if_path_does_not_exist(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Exists {
            path: file.path().to_path_buf(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert_eq!(res.payload[0], ResponseData::Exists(false));
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: file.path().to_path_buf(),
            canonicalize: false,
            resolve_file_type: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_back_metadata_on_file_if_exists(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: file.path().to_path_buf(),
            canonicalize: false,
            resolve_file_type: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 9,
                readonly: false,
                ..
            }
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_back_metadata_on_dir_if_exists(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: dir.path().to_path_buf(),
            canonicalize: false,
            resolve_file_type: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                readonly: false,
                ..
            }
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn metadata_should_send_back_metadata_on_symlink_if_exists(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: symlink.path().to_path_buf(),
            canonicalize: false,
            resolve_file_type: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Metadata {
                canonicalized_path: None,
                file_type: FileType::Symlink,
                readonly: false,
                ..
            }
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn metadata_should_include_canonicalized_path_if_flag_specified(#[future] session: Session) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: symlink.path().to_path_buf(),
            canonicalize: true,
            resolve_file_type: false,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::Metadata {
            canonicalized_path: Some(path),
            file_type: FileType::Symlink,
            readonly: false,
            ..
        } => assert_eq!(
            path,
            &file.path().canonicalize().unwrap(),
            "Symlink canonicalized path does not match referenced file"
        ),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn metadata_should_resolve_file_type_of_symlink_if_flag_specified(
    #[future] session: Session,
) {
    let mut session = session.await;
    let temp = TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();

    let req = Request::new(
        "test-tenant",
        vec![RequestData::Metadata {
            path: symlink.path().to_path_buf(),
            canonicalize: false,
            resolve_file_type: true,
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::Metadata {
            file_type: FileType::File,
            ..
        } => {}
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn proc_run_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            args: Vec::new(),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(&res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn proc_run_should_send_back_proc_start_on_success(#[future] session: Session) {
    let mut session = session.await;
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string()],
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(&res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_run_should_send_back_stdout_periodically_when_available(#[future] session: Session) {
    let mut session = session.await;
    // Run a program that echoes to stdout
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![
                ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string(),
                String::from("some stdout"),
            ],
        }],
    );

    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(&res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Gather two additional responses:
    //
    // 1. An indirect response for stdout
    // 2. An indirect response that is proc completing
    //
    // Note that order is not a guarantee, so we have to check that
    // we get one of each type of response
    let res1 = mailbox.next().await.expect("Missing first response");
    let res2 = mailbox.next().await.expect("Missing second response");

    let mut got_stdout = false;
    let mut got_done = false;

    let mut check_res = |res: &Response| {
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::ProcStdout { data, .. } => {
                assert_eq!(data, "some stdout", "Got wrong stdout");
                got_stdout = true;
            }
            ResponseData::ProcDone { success, .. } => {
                assert!(success, "Process should have completed successfully");
                got_done = true;
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    };

    check_res(&res1);
    check_res(&res2);
    assert!(got_stdout, "Missing stdout response");
    assert!(got_done, "Missing done response");
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_run_should_send_back_stderr_periodically_when_available(#[future] session: Session) {
    let mut session = session.await;
    // Run a program that echoes to stderr
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![
                ECHO_ARGS_TO_STDERR_SH.to_str().unwrap().to_string(),
                String::from("some stderr"),
            ],
        }],
    );

    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert!(
        matches!(&res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Gather two additional responses:
    //
    // 1. An indirect response for stderr
    // 2. An indirect response that is proc completing
    //
    // Note that order is not a guarantee, so we have to check that
    // we get one of each type of response
    let res1 = mailbox.next().await.expect("Missing first response");
    let res2 = mailbox.next().await.expect("Missing second response");

    let mut got_stderr = false;
    let mut got_done = false;

    let mut check_res = |res: &Response| {
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::ProcStderr { data, .. } => {
                assert_eq!(data, "some stderr", "Got wrong stderr");
                got_stderr = true;
            }
            ResponseData::ProcDone { success, .. } => {
                assert!(success, "Process should have completed successfully");
                got_done = true;
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    };

    check_res(&res1);
    check_res(&res2);
    assert!(got_stderr, "Missing stderr response");
    assert!(got_done, "Missing done response");
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_run_should_clear_process_from_state_when_done(#[future] session: Session) {
    let mut session = session.await;
    // Run a program that ends after a little bit
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("0.1")],
        }],
    );
    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Verify that the state has the process
    let res = session
        .send(Request::new("test-tenant", vec![RequestData::ProcList {}]))
        .await
        .unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::ProcEntries { entries } => assert_eq!(entries[0].id, id),
        x => panic!("Unexpected response: {:?}", x),
    }

    // Wait for process to finish
    let _ = mailbox.next().await.unwrap();

    // Verify that the state was cleared
    let res = session
        .send(Request::new("test-tenant", vec![RequestData::ProcList {}]))
        .await
        .unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::ProcEntries { entries } => assert!(entries.is_empty(), "Proc not cleared"),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn proc_run_should_clear_process_from_state_when_killed(#[future] session: Session) {
    let mut session = session.await;
    // Run a program that ends slowly
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
        }],
    );

    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Verify that the state has the process
    let res = session
        .send(Request::new("test-tenant", vec![RequestData::ProcList {}]))
        .await
        .unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::ProcEntries { entries } => assert_eq!(entries[0].id, id),
        x => panic!("Unexpected response: {:?}", x),
    }

    // Send kill signal
    let req = Request::new("test-tenant", vec![RequestData::ProcKill { id }]);
    let _ = session.send(req).await.unwrap();

    // Wait for the proc done
    let _ = mailbox.next().await.unwrap();

    // Verify that the state was cleared
    let res = session
        .send(Request::new("test-tenant", vec![RequestData::ProcList {}]))
        .await
        .unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    match &res.payload[0] {
        ResponseData::ProcEntries { entries } => assert!(entries.is_empty(), "Proc not cleared"),
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn proc_kill_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    // Send kill to a non-existent process
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcKill { id: 0xDEADBEEF }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");

    // Verify that we get an error
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn proc_kill_should_send_ok_and_done_responses_on_success(#[future] session: Session) {
    let mut session = session.await;
    // First, run a program that sits around (sleep for 1 second)
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
        }],
    );

    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");

    // Second, grab the id of the started process
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Third, send kill for process
    // NOTE: We cannot let the state get dropped as it results in killing
    //       the child process automatically; so, we clone another reference here
    let req = Request::new("test-tenant", vec![RequestData::ProcKill { id }]);
    let res = session.send(req).await.unwrap();
    match &res.payload[0] {
        ResponseData::Ok => {}
        x => panic!("Unexpected response: {:?}", x),
    }

    // Fourth, verify that the process completes
    let res = mailbox.next().await.unwrap();
    match &res.payload[0] {
        ResponseData::ProcDone { success, .. } => {
            assert!(!success, "Process should not have completed successfully");
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn proc_stdin_should_send_error_on_failure(#[future] session: Session) {
    let mut session = session.await;
    // Send stdin to a non-existent process
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcStdin {
            id: 0xDEADBEEF,
            data: String::from("some input"),
        }],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");

    // Verify that we get an error
    assert!(
        matches!(res.payload[0], ResponseData::Error(_)),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[tokio::test]
#[cfg_attr(windows, ignore)]
async fn proc_stdin_should_send_ok_on_success_and_properly_send_stdin_to_process(
    #[future] session: Session,
) {
    let mut session = session.await;

    // First, run a program that listens for stdin
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap().to_string()],
        }],
    );
    let mut mailbox = session.mail(req).await.unwrap();

    let res = mailbox.next().await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");

    // Second, grab the id of the started process
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Third, send stdin to the remote process
    // NOTE: We cannot let the state get dropped as it results in killing
    //       the child process; so, we clone another reference here
    let req = Request::new(
        "test-tenant",
        vec![RequestData::ProcStdin {
            id,
            data: String::from("hello world\n"),
        }],
    );
    let res = session.send(req).await.unwrap();
    match &res.payload[0] {
        ResponseData::Ok => {}
        x => panic!("Unexpected response: {:?}", x),
    }

    // Fourth, gather an indirect response that is stdout from echoing our stdin
    let res = mailbox.next().await.unwrap();
    match &res.payload[0] {
        ResponseData::ProcStdout { data, .. } => {
            assert_eq!(data, "hello world\n", "Mirrored data didn't match");
        }
        x => panic!("Unexpected response: {:?}", x),
    }
}

#[rstest]
#[tokio::test]
async fn proc_list_should_send_proc_entry_list(#[future] session: Session) {
    let mut session = session.await;
    // Run a process and get the list that includes that process
    // at the same time (using sleep of 1 second)
    let req = Request::new(
        "test-tenant",
        vec![
            RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
            },
            RequestData::ProcList {},
        ],
    );

    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 2, "Wrong payload size");

    // Grab the id of the started process
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Verify our process shows up in our entry list
    assert_eq!(
        res.payload[1],
        ResponseData::ProcEntries {
            entries: vec![RunningProcess {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                id,
            }],
        },
        "Unexpected response: {:?}",
        res.payload[0]
    );
}

#[rstest]
#[tokio::test]
async fn system_info_should_send_system_info_based_on_binary(#[future] session: Session) {
    let mut session = session.await;
    let req = Request::new("test-tenant", vec![RequestData::SystemInfo {}]);
    let res = session.send(req).await.unwrap();
    assert_eq!(res.payload.len(), 1, "Wrong payload size");
    assert_eq!(
        res.payload[0],
        ResponseData::SystemInfo {
            family: env::consts::FAMILY.to_string(),
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            current_dir: env::current_dir().unwrap_or_default(),
            main_separator: std::path::MAIN_SEPARATOR,
        },
        "Unexpected response: {:?}",
        res.payload[0]
    );
}
