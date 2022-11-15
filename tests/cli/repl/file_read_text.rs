use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;
use test_log::test;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[test(tokio::test)]
async fn should_support_json_output(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_read_text",
            "path": file.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "text",
            "data": FILE_CONTENTS.to_string()
        }),
        "JSON: {res}"
    );
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_output_for_error(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_read_text",
            "path": file.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    assert_eq!(res["payload"]["kind"], "not_found", "JSON: {res}");
}
