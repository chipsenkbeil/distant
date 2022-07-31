use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

const APPENDED_FILE_CONTENTS: &str = r#"
even more
file contents
"#;

#[rstest]
#[tokio::test]
async fn should_support_json_output(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str(FILE_CONTENTS).unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_append_text",
            "path": file.to_path_buf(),
            "text": APPENDED_FILE_CONTENTS.to_string(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(
        res["payload"],
        json!({
            "type": "ok"
        })
    );

    // NOTE: We wait a little bit to give the OS time to fully write to file
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Because we're talking to a local server, we can verify locally
    file.assert(format!("{}{}", FILE_CONTENTS, APPENDED_FILE_CONTENTS));
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-dir").child("missing-file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "file_append_text",
            "path": file.to_path_buf(),
            "text": APPENDED_FILE_CONTENTS.to_string(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");

    // Because we're talking to a local server, we can verify locally
    file.assert(predicates::path::missing());
}
