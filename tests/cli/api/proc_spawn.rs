use rstest::*;
use serde_json::json;
use test_log::test;

use crate::cli::scripts::*;
use crate::common::fixtures::*;

fn make_cmd(args: Vec<&str>) -> String {
    format!(
        r#"{} {} {}"#,
        *SCRIPT_RUNNER,
        *SCRIPT_RUNNER_ARG,
        args.join(" ")
    )
}

fn trim(arr: &Vec<serde_json::Value>) -> &[serde_json::Value] {
    let arr = arr.as_slice();

    if arr.is_empty() {
        return arr;
    }

    let mut start = 0;
    let mut end = arr.len() - 1;
    let mut i = start;

    fn is_whitespace(value: &serde_json::Value) -> bool {
        value == b' ' || value == b'\t' || value == b'\r' || value == b'\n'
    }

    // Trim from front
    while start < end {
        if is_whitespace(&arr[i]) {
            start = i + 1;
            i += 1;
        } else {
            break;
        }
    }

    i = end;

    // Trim from back
    while end > start {
        if is_whitespace(&arr[i]) {
            end = i - 1;
            i -= 1;
        } else {
            break;
        }
    }

    &arr[start..=end]
}

// Trim and compare value to string
fn check_value_as_str(value: &serde_json::Value, other: &str) {
    let arr = trim(value.as_array().expect("value should be a byte array"));

    if arr != other.as_bytes() {
        let s = arr
            .iter()
            .map(|value| {
                (value
                    .as_u64()
                    .expect("Invalid array value, expected number") as u8) as char
            })
            .collect::<String>();
        panic!("Expected '{other}', but got '{s}'");
    }
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_to_execute_program_and_return_exit_status(
    mut api_process: CtxCommand<ApiProcess>,
) {
    validate_authentication(&mut api_process).await;

    let cmd = make_cmd(vec![ECHO_ARGS_TO_STDOUT.to_str().unwrap()]);

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "pty": null,
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_spawned", "JSON: {res}");
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_to_capture_and_print_stdout(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let cmd = make_cmd(vec![ECHO_ARGS_TO_STDOUT.to_str().unwrap(), "some output"]);

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "pty": null,
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_spawned", "JSON: {res}");

    // Wait for output to show up (for stderr)
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_stdout", "JSON: {res}");
    check_value_as_str(&res["payload"]["data"], "some output");

    // Now we wait for the process to complete
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_done", "JSON: {res}");
    assert_eq!(res["payload"]["success"], true, "JSON: {res}");
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_to_capture_and_print_stderr(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let cmd = make_cmd(vec![ECHO_ARGS_TO_STDERR.to_str().unwrap(), "some output"]);

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "pty": null,
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_spawned", "JSON: {res}");

    // Wait for output to show up (for stderr)
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_stderr", "JSON: {res}");
    check_value_as_str(&res["payload"]["data"], "some output");

    // Now we wait for the process to complete
    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_done", "JSON: {res}");
    assert_eq!(res["payload"]["success"], true, "JSON: {res}");
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_to_forward_stdin_to_remote_process(
    mut api_process: CtxCommand<ApiProcess>,
) {
    validate_authentication(&mut api_process).await;

    let cmd = make_cmd(vec![ECHO_STDIN_TO_STDOUT.to_str().unwrap()]);

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "pty": null,
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_spawned", "JSON: {res}");

    // Write output to stdin of process to trigger getting it back as stdout
    let proc_id = res["payload"]["id"]
        .as_u64()
        .expect("Invalid proc id value");
    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "proc_stdin",
            "id": proc_id,
            "data": b"some output\n",
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "ok", "JSON: {res}");

    let res = api_process.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "proc_stdout", "JSON: {res}");
    check_value_as_str(&res["payload"]["data"], "some output");

    // Now kill the process and wait for it to complete
    let id = rand::random::<u64>().to_string();
    let res = api_process
        .write_and_read_json(json!({
            "id": id,
            "payload": {
                "type": "proc_kill",
                "id": proc_id,
            },
        }))
        .await
        .unwrap()
        .unwrap();

    // If the first response we get is proc_done, then we don't care bout the kill result
    // as it can be an error if the process ended before the kill happened
    //
    // NOTE: The above is a situation in Windows, but I've not seen it happen with Mac/Linux.
    if res["payload"]["type"] == "ok" {
        let res = api_process.read_json_from_stdout().await.unwrap().unwrap();
        assert_eq!(
            res["payload"]["type"], "proc_done",
            "Did not receive proc_done from killed process: {res}"
        );
    } else {
        assert_eq!(res["payload"]["type"], "proc_done", "JSON: {res}");
    }
}

#[rstest]
#[test(tokio::test)]
async fn should_support_json_output_for_error(mut api_process: CtxCommand<ApiProcess>) {
    validate_authentication(&mut api_process).await;

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "proc_spawn",
            "cmd": DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            "pty": null,
        },
    });

    let res = api_process.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id, "JSON: {res}");
    assert_eq!(res["payload"]["type"], "error", "JSON: {res}");
    assert_eq!(res["payload"]["kind"], "not_found", "JSON: {res}");
}
