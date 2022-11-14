use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;

#[rstest]
#[tokio::test]
async fn should_support_json_search_filesystem_using_query(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let root = assert_fs::TempDir::new().unwrap();
    root.child("file1.txt").write_str("some file text").unwrap();
    root.child("file2.txt")
        .write_str("lines\nof\ntextual\ninformation")
        .unwrap();
    root.child("file3.txt").write_str("more content").unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "search",
            "query": {
                "paths": [root.path().to_string_lossy()],
                "target": "contents",
                "condition": {"type": "regex", "value": "ua"},
            },
        },
    });

    // Submit search request and get back started confirmation
    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    // Get id from started confirmation
    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "search_started", "JSON: {res}");
    let search_id = res["payload"]["id"]
        .as_u64()
        .expect("id missing or not number");

    // Get search results back
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();
    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "search_results",
            "id": search_id,
            "matches": [
                {
                    "type": "contents",
                    "path": root.child("file2.txt").to_string_lossy(),
                    "lines": {
                        "type": "text",
                        "value": "textual\n",
                    },
                    "line_number": 3,
                    "absolute_offset": 9,
                    "submatches": [
                        {
                            "match": {
                                "type": "text",
                                "value": "ua",
                            },
                            "start": 4,
                            "end": 6,
                        }
                    ],
                },
            ]
        }),
        "JSON: {res}"
    );

    // Get search completion confirmation
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();
    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "search_done",
            "id": search_id,
        }),
        "JSON: {res}"
    );
}
