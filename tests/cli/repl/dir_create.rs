use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;
use serde_json::json;
use test_log::test;

#[rstest]
#[test(tokio::test)]
async fn should_support_json_output(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_create",
            "path": dir.to_path_buf(),
            "all": false,
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

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_creating_missing_parent_directories_if_specified(
    mut json_repl: CtxCommand<Repl>,
) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir1").child("dir2");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_create",
            "path": dir.to_path_buf(),
            "all": true,
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

    dir.assert(predicate::path::exists());
    dir.assert(predicate::path::is_dir());
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_output_for_error(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("missing-dir").child("dir");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "dir_create",
            "path": dir.to_path_buf(),
            "all": false,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    assert_eq!(res["payload"]["kind"], "not_found", "JSON: {res}");

    dir.assert(predicate::path::missing());
}
