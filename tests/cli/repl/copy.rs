use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;
use serde_json::json;

const FILE_CONTENTS: &str = r#"
some text
on multiple lines
that is a file's contents
"#;

#[rstest]
#[tokio::test]
async fn should_support_json_copying_file(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("file");
    src.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("file2");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "copy",
            "src": src.to_path_buf(),
            "dst": dst.to_path_buf(),
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

    src.assert(predicate::path::exists());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
#[tokio::test]
async fn should_support_json_copying_nonempty_directory(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    // Make a non-empty directory
    let src = temp.child("dir");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str(FILE_CONTENTS).unwrap();

    let dst = temp.child("dir2");
    let dst_file = dst.child("file");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "copy",
            "src": src.to_path_buf(),
            "dst": dst.to_path_buf(),
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

    src_file.assert(predicate::path::exists());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("dir");
    let dst = temp.child("dir2");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "copy",
            "src": src.to_path_buf(),
            "dst": dst.to_path_buf(),
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");

    src.assert(predicate::path::missing());
    dst.assert(predicate::path::missing());
}
