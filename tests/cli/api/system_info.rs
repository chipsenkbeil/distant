use std::env;

use rstest::*;
use serde_json::json;
use test_log::test;

use crate::common::fixtures::*;

#[rstest]
#[test(tokio::test)]
async fn should_support_json_system_info(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": { "type": "system_info" },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(
        res["payload"],
        json!({
            "type": "system_info",
            "family": env::consts::FAMILY.to_string(),
            "os": env::consts::OS.to_string(),
            "arch": env::consts::ARCH.to_string(),
            "current_dir": env::current_dir().unwrap_or_default(),
            "main_separator": std::path::MAIN_SEPARATOR.to_string(),
            "username": whoami::username(),
            "shell": if cfg!(windows) {
                std::env::var("ComSpec").unwrap_or_else(|_| String::from("cmd.exe"))
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"))
            }
        }),
        "JSON: {res}"
    );
}
