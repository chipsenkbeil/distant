use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;
use serde_json::json;

#[rstest]
#[tokio::test]
async fn should_support_json_removing_file(mut json_repl: CtxCommand<Repl>) {
    let temp = assert_fs::TempDir::new().unwrap();

    let file = temp.child("file");
    file.touch().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "remove",
            "path": file.to_path_buf(),
            "force": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        }),
        "JSON: {res}"
    );

    file.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn should_support_json_removing_empty_directory(mut json_repl: CtxCommand<Repl>) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "remove",
            "path": dir.to_path_buf(),
            "force": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        }),
        "JSON: {res}"
    );

    dir.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn should_support_json_removing_nonempty_directory_if_force_specified(
    mut json_repl: CtxCommand<Repl>,
) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make an empty directory
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "remove",
            "path": dir.to_path_buf(),
            "force": true,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        }),
        "JSON: {res}"
    );

    dir.assert(predicate::path::missing());
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: CtxCommand<Repl>) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory so we fail to remove it
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "remove",
            "path": dir.to_path_buf(),
            "force": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    assert!(
        res["payload"]["kind"] == "other" || res["payload"]["kind"] == "unknown",
        "error kind was neither other or unknown; JSON: {res}"
    );

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}
