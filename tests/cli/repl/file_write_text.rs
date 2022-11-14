use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[tokio::test]
async fn should_support_json_output(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_write_text",
            "path": file.to_path_buf(),
            "text": FILE_CONTENTS.to_string(),
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

    // NOTE: We wait a little bit to give the OS time to fully write to file
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Because we're talking to a local server, we can verify locally
    file.assert(FILE_CONTENTS);
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_write_text",
            "path": file.to_path_buf(),
            "text": FILE_CONTENTS.to_string(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    assert_eq!(res["payload"]["kind"], "not_found", "JSON: {res}");

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}
