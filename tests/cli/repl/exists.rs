use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;

#[rstest]
#[tokio::test]
async fn should_support_json_true_if_exists(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Create file
    let file = temp.child("file");
    file.touch().unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "exists",
            "path": file.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "exists",
            "value": true,
        })
    );
}

#[rstest]
#[tokio::test]
async fn should_support_json_false_if_not_exists(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Don't create file
    let file = temp.child("file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "exists",
            "path": file.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "exists",
            "value": false,
        })
    );
}
