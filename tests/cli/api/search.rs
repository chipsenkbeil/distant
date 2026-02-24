//! Integration tests for the `search` JSON API endpoint.
//!
//! Tests searching file contents with a regex query and verifying the multi-step
//! search protocol (started, results, done).

use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;
use test_log::test;

use distant_test_harness::manager::*;

#[rstest]
#[test(tokio::test)]
async fn should_support_json_search_filesystem_using_query(
    mut api_process: CtxCommand<ApiProcess>,
) {
    validate_authentication(&mut api_process).await;

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
    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    // Get id from started confirmation
    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "search_started", "JSON: {res}");
    let search_id = res["payload"]["id"]
        .as_u64()
        .expect("id missing or not number");

    // Get search results back
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();
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
                    "lines": "textual\n",
                    "line_number": 3,
                    "absolute_offset": 9,
                    "submatches": [
                        {
                            "match": "ua",
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
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();
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
