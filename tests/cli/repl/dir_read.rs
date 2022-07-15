use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;
use std::path::PathBuf;

/// Creates a directory in the form
///
/// $TEMP/
/// $TEMP/dir1/
/// $TEMP/dir1/dira/
/// $TEMP/dir1/dirb/
/// $TEMP/dir1/dirb/file1
/// $TEMP/dir1/file1
/// $TEMP/dir1/file2
/// $TEMP/dir2/
/// $TEMP/dir2/dira/
/// $TEMP/dir2/dirb/
/// $TEMP/dir2/dirb/file1
/// $TEMP/dir2/file1
/// $TEMP/dir2/file2
/// $TEMP/file1
/// $TEMP/file2
fn make_directory() -> assert_fs::TempDir {
    let temp = assert_fs::TempDir::new().unwrap();

    // $TEMP/file1
    // $TEMP/file2
    temp.child("file1").touch().unwrap();
    temp.child("file2").touch().unwrap();

    // $TEMP/dir1/
    // $TEMP/dir1/file1
    // $TEMP/dir1/file2
    let dir1 = temp.child("dir1");
    dir1.create_dir_all().unwrap();
    dir1.child("file1").touch().unwrap();
    dir1.child("file2").touch().unwrap();

    // $TEMP/dir1/dira/
    let dir1_dira = dir1.child("dira");
    dir1_dira.create_dir_all().unwrap();

    // $TEMP/dir1/dirb/
    // $TEMP/dir1/dirb/file1
    let dir1_dirb = dir1.child("dirb");
    dir1_dirb.create_dir_all().unwrap();
    dir1_dirb.child("file1").touch().unwrap();

    // $TEMP/dir2/
    // $TEMP/dir2/file1
    // $TEMP/dir2/file2
    let dir2 = temp.child("dir2");
    dir2.create_dir_all().unwrap();
    dir2.child("file1").touch().unwrap();
    dir2.child("file2").touch().unwrap();

    // $TEMP/dir2/dira/
    let dir2_dira = dir2.child("dira");
    dir2_dira.create_dir_all().unwrap();

    // $TEMP/dir2/dirb/
    // $TEMP/dir2/dirb/file1
    let dir2_dirb = dir2.child("dirb");
    dir2_dirb.create_dir_all().unwrap();
    dir2_dirb.child("file1").touch().unwrap();

    temp
}

#[rstest]
#[tokio::test]
async fn should_support_json_output(mut json_repl: Repl) {
    let temp = make_directory();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_read",
            "path": temp.to_path_buf(),
            "depth": 1,
            "absolute": false,
            "canonicalize": false,
            "include_root": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "dir_entries",
            "entries": [
                {"path": PathBuf::from("dir1"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("dir2"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("file1"), "file_type": "file", "depth": 1},
                {"path": PathBuf::from("file2"), "file_type": "file", "depth": 1},
            ],
            "errors": [],
        })
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_returning_absolute_paths_if_specified(mut json_repl: Repl) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so the absolute path
    //       provided is our canonicalized root path prepended
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_read",
            "path": temp.to_path_buf(),
            "depth": 1,
            "absolute": true,
            "canonicalize": false,
            "include_root": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "dir_entries",
            "entries": [
                {"path": root_path.join("dir1"), "file_type": "dir", "depth": 1},
                {"path": root_path.join("dir2"), "file_type": "dir", "depth": 1},
                {"path": root_path.join("file1"), "file_type": "file", "depth": 1},
                {"path": root_path.join("file2"), "file_type": "file", "depth": 1},
            ],
            "errors": [],
        })
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_returning_all_files_and_directories_if_depth_is_0(
    mut json_repl: Repl,
) {
    let temp = make_directory();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_read",
            "path": temp.to_path_buf(),
            "depth": 0,
            "absolute": false,
            "canonicalize": false,
            "include_root": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "dir_entries",
            "entries": [
                {"path": PathBuf::from("dir1"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("dir1").join("dira"), "file_type": "dir", "depth": 2},
                {"path": PathBuf::from("dir1").join("dirb"), "file_type": "dir", "depth": 2},
                {"path": PathBuf::from("dir1").join("dirb").join("file1"), "file_type": "file", "depth": 3},
                {"path": PathBuf::from("dir1").join("file1"), "file_type": "file", "depth": 2},
                {"path": PathBuf::from("dir1").join("file2"), "file_type": "file", "depth": 2},
                {"path": PathBuf::from("dir2"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("dir2").join("dira"), "file_type": "dir", "depth": 2},
                {"path": PathBuf::from("dir2").join("dirb"), "file_type": "dir", "depth": 2},
                {"path": PathBuf::from("dir2").join("dirb").join("file1"), "file_type": "file", "depth": 3},
                {"path": PathBuf::from("dir2").join("file1"), "file_type": "file", "depth": 2},
                {"path": PathBuf::from("dir2").join("file2"), "file_type": "file", "depth": 2},
                {"path": PathBuf::from("file1"), "file_type": "file", "depth": 1},
                {"path": PathBuf::from("file2"), "file_type": "file", "depth": 1},
            ],
            "errors": [],
        })
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_including_root_directory_if_specified(mut json_repl: Repl) {
    let temp = make_directory();

    // NOTE: Our root path is always canonicalized, so yielded entry
    //       is the canonicalized version
    let root_path = temp.to_path_buf().canonicalize().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_read",
            "path": temp.to_path_buf(),
            "depth": 1,
            "absolute": false,
            "canonicalize": false,
            "include_root": true,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "dir_entries",
            "entries": [
                {"path": root_path, "file_type": "dir", "depth": 0},
                {"path": PathBuf::from("dir1"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("dir2"), "file_type": "dir", "depth": 1},
                {"path": PathBuf::from("file1"), "file_type": "file", "depth": 1},
                {"path": PathBuf::from("file2"), "file_type": "file", "depth": 1},
            ],
            "errors": [],
        })
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let temp = make_directory();
    let dir = temp.child("missing-dir");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_read",
            "path": dir.to_path_buf(),
            "depth": 1,
            "absolute": false,
            "canonicalize": false,
            "include_root": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");
}
