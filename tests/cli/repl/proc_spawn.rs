use crate::cli::fixtures::*;
use assert_fs::prelude::*;
use once_cell::sync::Lazy;
use rstest::*;
use serde_json::json;

static TEMP_SCRIPT_DIR: Lazy<assert_fs::TempDir> = Lazy::new(|| assert_fs::TempDir::new().unwrap());
static SCRIPT_RUNNER: Lazy<String> = Lazy::new(|| String::from("bash"));

static ECHO_ARGS_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
            #/usr/bin/env bash
            printf "%s" "$*"
        "#
        ))
        .unwrap();
    script
});

static ECHO_ARGS_TO_STDERR_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
    script
        .write_str(indoc::indoc!(
            r#"
            #/usr/bin/env bash
            printf "%s" "$*" 1>&2
        "#
        ))
        .unwrap();
    script
});

static ECHO_STDIN_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
            #/usr/bin/env bash
            while IFS= read; do echo "$REPLY"; done
        "#
        ))
        .unwrap();
    script
});

static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

#[rstest]
#[tokio::test]
async fn should_support_json_to_execute_program_and_return_exit_status(mut json_repl: Repl) {
    let cmd = format!(
        "{} {}",
        *SCRIPT_RUNNER,
        ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap()
    );

    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "persist": false,
            "pty": null,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "proc_spawned");
}

#[rstest]
#[tokio::test]
async fn should_support_json_to_capture_and_print_stdout(mut json_repl: Repl) {
    let cmd = format!(
        "{} {} some output",
        *SCRIPT_RUNNER,
        ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap(),
    );

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "persist": false,
            "pty": null,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_spawned");

    // Wait for output to show up (for stderr)
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_stdout");
    assert_eq!(
        res["payload"]["data"]
            .as_array()
            .expect("data should be a byte array"),
        b"some output"
    );

    // Now we wait for the process to complete
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_done");
    assert_eq!(res["payload"]["success"], true);
}

#[rstest]
#[tokio::test]
async fn should_support_json_to_capture_and_print_stderr(mut json_repl: Repl) {
    let cmd = format!(
        "{} {} some output",
        *SCRIPT_RUNNER,
        ECHO_ARGS_TO_STDERR_SH.to_str().unwrap(),
    );

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "persist": false,
            "pty": null,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_spawned");

    // Wait for output to show up (for stderr)
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_stderr");
    assert_eq!(
        res["payload"]["data"]
            .as_array()
            .expect("data should be a byte array"),
        b"some output"
    );

    // Now we wait for the process to complete
    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_done");
    assert_eq!(res["payload"]["success"], true);
}

#[rstest]
#[tokio::test]
async fn should_support_json_to_forward_stdin_to_remote_process(mut json_repl: Repl) {
    let cmd = format!(
        "{} {}",
        *SCRIPT_RUNNER,
        ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap(),
    );

    // Spawn the process
    let origin_id = rand::random::<u64>().to_string();
    let req = json!({
        "id": origin_id,
        "payload": {
            "type": "proc_spawn",
            "cmd": cmd,
            "persist": false,
            "pty": null,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_spawned");

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
            "data": b"some output",
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "ok");

    let res = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    assert_eq!(res["origin_id"], origin_id);
    assert_eq!(res["payload"]["type"], "proc_stdout");
    assert_eq!(
        res["payload"]["data"]
            .as_array()
            .expect("data should be a byte array"),
        b"some output"
    );

    // Now kill the process and wait for it to complete
    let id = rand::random::<u64>().to_string();
    let res_1 = json_repl
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
    let res_2 = json_repl.read_json_from_stdout().await.unwrap().unwrap();

    // The order of responses may be different (kill could come before ok), so we need
    // to check that we get one of each type
    let got_ok = res_1["payload"]["type"] == "ok" || res_2["payload"]["type"] == "ok";
    let got_done =
        res_1["payload"]["type"] == "proc_done" || res_2["payload"]["type"] == "proc_done";

    if res_1["payload"]["type"] == "ok" {
        assert_eq!(res_1["origin_id"], id);
    } else {
        assert_eq!(res_1["origin_id"], origin_id);
    }

    if res_2["payload"]["type"] == "ok" {
        assert_eq!(res_2["origin_id"], id);
    } else {
        assert_eq!(res_2["origin_id"], origin_id);
    }

    assert!(got_ok, "Did not receive ok from proc_kill");
    assert!(got_done, "Did not receive proc_done from killed process");
}

#[rstest]
#[tokio::test]
async fn should_support_json_output_for_error(mut json_repl: Repl) {
    let id = rand::random::<u64>().to_string();
    let req = json!({
        "id": id,
        "payload": {
            "type": "proc_spawn",
            "cmd": DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            "persist": false,
            "pty": null,
        },
    });

    let res = json_repl.write_and_read_json(req).await.unwrap().unwrap();

    assert_eq!(res["origin_id"], id);
    assert_eq!(res["payload"]["type"], "error");
    assert_eq!(res["payload"]["kind"], "not_found");
}