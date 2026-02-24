//! Integration tests for the `set_permissions` JSON API endpoint.
//!
//! Tests setting file permissions (readonly, Unix mode bits) and error handling
//! for nonexistent paths. Uses JSON requests sent to an `ApiProcess`.

#![allow(unexpected_cfgs)]

use assert_fs::prelude::*;
use rstest::*;
use serde_json::json;
use test_log::test;

use distant_test_harness::manager::*;

#[cfg(unix)]
#[rstest]
#[test(tokio::test)]
async fn should_support_json_set_permissions_readonly(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("test content").unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "set_permissions",
            "path": file.to_path_buf(),
            "permissions": {
                "owner_read": true,
                "owner_write": false,
                "owner_exec": false,
                "group_read": true,
                "group_write": false,
                "group_exec": false,
                "other_read": true,
                "other_write": false,
                "other_exec": false,
            },
            "options": {},
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "ok", "JSON: {res}");

    // Verify the file is actually readonly
    let meta = std::fs::metadata(file.path()).unwrap();
    assert!(meta.permissions().readonly(), "File should be readonly");
}

#[cfg(unix)]
#[rstest]
#[test(tokio::test)]
async fn should_support_json_set_permissions_with_unix_mode(
    mut api_process: CtxCommand<ApiProcess>,
) {
    validate_authentication(&mut api_process).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("test content").unwrap();

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "set_permissions",
            "path": file.to_path_buf(),
            "permissions": {
                "owner_read": true,
                "owner_write": false,
                "owner_exec": false,
                "group_read": false,
                "group_write": false,
                "group_exec": false,
                "other_read": false,
                "other_write": false,
                "other_exec": false,
            },
            "options": {},
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "ok", "JSON: {res}");

    // Verify the file permissions
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o400, "Expected mode 0o400, got {:o}", mode);
}

#[rstest]
#[test(tokio::test)]
async fn should_fail_for_nonexistent_path(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let temp = assert_fs::TempDir::new().unwrap();
    let missing = temp.child("nonexistent");

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "set_permissions",
            "path": missing.to_path_buf(),
            "permissions": {
                "owner_read": true,
                "owner_write": false,
            },
            "options": {},
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    // set_permissions wraps OS errors as "permission_denied" even for non-existent paths
    assert_eq!(res["payload"]["kind"], "permission_denied", "JSON: {res}");
}
