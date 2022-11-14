use crate::cli::fixtures::*;
use distant_core::data::{Capabilities, Capability};
use rstest::*;
use serde_json::json;

#[rstest]
#[tokio::test]
async fn should_support_json_capabilities(mut json_repl: CtxCommand<Repl>) {
    validate_authentication(&mut json_repl).await;

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": { "type": "capabilities" },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "capabilities", "JSON: {res}");

    let supported: Capabilities = res["payload"]["supported"]
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
    assert_eq!(supported, Capabilities::all());
}
