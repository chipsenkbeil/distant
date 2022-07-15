use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::{json, Value};

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[tokio::test]
async fn should_support_json_metadata_for_file(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.write_str(FILE_CONTENTS).unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "metadata",
            "path": file.to_path_buf(),
            "canonicalize": false,
            "resolve_file_type": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "metadata");
    assert_eq!(res["payload"]["canonicalized_path"], Value::Null);
    assert_eq!(res["payload"]["file_type"], "file");
    assert_eq!(res["payload"]["readonly"], false);
}

#[rstest]
#[tokio::test]
async fn should_support_json_metadata_for_directory(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "metadata",
            "path": dir.to_path_buf(),
            "canonicalize": false,
            "resolve_file_type": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "metadata");
    assert_eq!(res["payload"]["canonicalized_path"], Value::Null);
    assert_eq!(res["payload"]["file_type"], "dir");
    assert_eq!(res["payload"]["readonly"], false);
}

#[rstest]
#[tokio::test]
async fn should_support_json_metadata_for_including_a_canonicalized_path(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "metadata",
            "path": link.to_path_buf(),
            "canonicalize": true,
            "resolve_file_type": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "metadata");
    assert_eq!(
        res["payload"]["canonicalized_path"],
        json!(file.path().canonicalize().unwrap())
    );
    assert_eq!(res["payload"]["file_type"], "symlink");
    assert_eq!(res["payload"]["readonly"], false);
}

#[rstest]
#[tokio::test]
async fn should_support_json_metadata_for_resolving_file_type_of_symlink(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let link = temp.child("link");
    link.symlink_to_file(file.path()).unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "metadata",
            "path": link.to_path_buf(),
            "canonicalize": true,
            "resolve_file_type": true,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "metadata");
    assert_eq!(res["payload"]["file_type"], "file");
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "metadata",
            "path": file.to_path_buf(),
            "canonicalize": false,
            "resolve_file_type": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");
}