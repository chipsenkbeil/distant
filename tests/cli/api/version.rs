use distant_core::protocol::{Capabilities, Capability, SemVer, PROTOCOL_VERSION};
use rstest::*;
use serde_json::json;
use test_log::test;

use crate::common::fixtures::*;

#[rstest]
#[test(tokio::test)]
async fn should_support_json_capabilities(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": { "type": "version" },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "version", "JSON: {res}");

    let protocol_version: SemVer =
        serde_json::from_value(res["payload"]["protocol_version"].clone()).unwrap();
    assert_eq!(protocol_version, PROTOCOL_VERSION);

    let capabilities: Capabilities = res["payload"]["capabilities"]
        .as_array()
        .expect("Field 'supported' was not an array")
        .iter()
        .map(|value| {
            serde_json::from_value::<Capability>(value.clone())
                .expect("Could not read array value as capability")
        })
        .collect();

    // NOTE: Our local server api should always support all capabilities since it is the reference
    //       implementation for our api
    assert_eq!(capabilities, Capabilities::all());
}
