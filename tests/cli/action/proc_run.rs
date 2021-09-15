use crate::cli::{
    fixtures::*,
    utils::{distant_subcommand, friendly_recv_line, random_tenant, spawn_line_reader},
};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;
use std::{io::Write, time::Duration};

lazy_static::lazy_static! {
    static ref TEMP_SCRIPT_DIR: assert_fs::TempDir = assert_fs::TempDir::new().unwrap();
    static ref SCRIPT_RUNNER: String = String::from("bash");

    static ref ECHO_ARGS_TO_STDOUT_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
        script.write_str(indoc::indoc!(r#"
            #/usr/bin/env bash
            printf "%s" "$@"
        "#)).unwrap();
        script
    };

    static ref ECHO_ARGS_TO_STDERR_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
        script.write_str(indoc::indoc!(r#"
            #/usr/bin/env bash
            printf "%s" "$@" 1>&2
        "#)).unwrap();
        script
    };

    static ref ECHO_STDIN_TO_STDOUT_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
        script.write_str(indoc::indoc!(r#"
            #/usr/bin/env bash
            while IFS= read; do echo "$REPLY"; done
        "#)).unwrap();
        script
    };

    static ref EXIT_CODE_SH: assert_fs::fixture::ChildPath = {
        let script = TEMP_SCRIPT_DIR.child("exit_code.sh");
        script.write_str(indoc::indoc!(r#"
            #!/usr/bin/env bash
            exit "$1"
        "#)).unwrap();
        script
    };

    static ref DOES_NOT_EXIST_BIN: assert_fs::fixture::ChildPath =
        TEMP_SCRIPT_DIR.child("does_not_exist_bin");
}

macro_rules! next_two_msgs {
    ($rx:expr) => {{
        let out = friendly_recv_line($rx, Duration::from_secs(1)).unwrap();
        let res1: Response = serde_json::from_str(&out).unwrap();
        let out = friendly_recv_line($rx, Duration::from_secs(1)).unwrap();
        let res2: Response = serde_json::from_str(&out).unwrap();
        (res1, res2)
    }};
}

#[rstest]
fn should_execute_program_and_return_exit_status(mut action_cmd: Command) {
    // distant action proc-run -- {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(EXIT_CODE_SH.to_str().unwrap())
        .arg("0")
        .assert()
        .success()
        .stdout("")
        .stderr("");
}

#[rstest]
fn should_capture_and_print_stdout(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap())
        .arg("hello world")
        .assert()
        .success()
        .stdout("hello world")
        .stderr("");
}

#[rstest]
fn should_capture_and_print_stderr(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(ECHO_ARGS_TO_STDERR_SH.to_str().unwrap())
        .arg("hello world")
        .assert()
        .success()
        .stdout("")
        .stderr("hello world");
}

#[rstest]
fn should_forward_stdin_to_remote_process(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap())
        .write_stdin("hello world\n")
        .assert()
        .success()
        .stdout("hello world\n")
        .stderr("");
}

#[rstest]
fn reflect_the_exit_code_of_the_process(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(EXIT_CODE_SH.to_str().unwrap())
        .arg("99")
        .assert()
        .code(99)
        .stdout("")
        .stderr("");
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(DOES_NOT_EXIST_BIN.to_str().unwrap())
        .assert()
        .code(ExitCode::DataErr.to_i32())
        .stdout("")
        .stderr("");
}

#[rstest]
fn should_support_json_to_execute_program_and_return_exit_status(mut action_cmd: Command) {
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string()],
        }],
    };

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", serde_json::to_string(&req).unwrap()))
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert!(
        matches!(res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0],
    );
}

#[rstest]
fn should_support_json_to_capture_and_print_stdout(ctx: &'_ DistantServerCtx) {
    let output = String::from("some output");
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![
                ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string(),
                output.to_string(),
            ],
        }],
    };

    // distant action --format json --interactive
    let mut child = distant_subcommand(ctx, "action")
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = spawn_line_reader(child.stdout.take().unwrap());
    let stderr = spawn_line_reader(child.stderr.take().unwrap());

    // Send our request as json
    let req_string = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin.write_all(req_string.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Get the indicator of a process started (first line returned can take ~7 seconds due to the
    // handshake cost)
    let out =
        friendly_recv_line(&stdout, Duration::from_secs(30)).expect("Failed to get proc start");
    let res: Response = serde_json::from_str(&out).unwrap();
    assert!(
        matches!(res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Get stdout from process and verify it
    let out =
        friendly_recv_line(&stdout, Duration::from_secs(1)).expect("Failed to get proc stdout");
    let res: Response = serde_json::from_str(&out).unwrap();
    match &res.payload[0] {
        ResponseData::ProcStdout { data, .. } => assert_eq!(data, &output),
        x => panic!("Unexpected response: {:?}", x),
    };

    // Get the indicator of a process completion
    let out = friendly_recv_line(&stdout, Duration::from_secs(1)).expect("Failed to get proc done");
    let res: Response = serde_json::from_str(&out).unwrap();
    match &res.payload[0] {
        ResponseData::ProcDone { success, .. } => {
            assert!(success, "Process failed unexpectedly");
        }
        x => panic!("Unexpected response: {:?}", x),
    };

    // Verify that we received nothing on stderr channel
    assert!(
        stderr.try_recv().is_err(),
        "Unexpectedly got result on stderr channel"
    );
}

#[rstest]
fn should_support_json_to_capture_and_print_stderr(ctx: &'_ DistantServerCtx) {
    let output = String::from("some output");
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![
                ECHO_ARGS_TO_STDERR_SH.to_str().unwrap().to_string(),
                output.to_string(),
            ],
        }],
    };

    // distant action --format json --interactive
    let mut child = distant_subcommand(ctx, "action")
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = spawn_line_reader(child.stdout.take().unwrap());
    let stderr = spawn_line_reader(child.stderr.take().unwrap());

    // Send our request as json
    let req_string = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin.write_all(req_string.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Get the indicator of a process started (first line returned can take ~7 seconds due to the
    // handshake cost)
    let out =
        friendly_recv_line(&stdout, Duration::from_secs(30)).expect("Failed to get proc start");
    let res: Response = serde_json::from_str(&out).unwrap();
    assert!(
        matches!(res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0]
    );

    // Get stderr from process and verify it
    let out =
        friendly_recv_line(&stdout, Duration::from_secs(1)).expect("Failed to get proc stderr");
    let res: Response = serde_json::from_str(&out).unwrap();
    match &res.payload[0] {
        ResponseData::ProcStderr { data, .. } => assert_eq!(data, &output),
        x => panic!("Unexpected response: {:?}", x),
    };

    // Get the indicator of a process completion
    let out = friendly_recv_line(&stdout, Duration::from_secs(1)).expect("Failed to get proc done");
    let res: Response = serde_json::from_str(&out).unwrap();
    match &res.payload[0] {
        ResponseData::ProcDone { success, .. } => {
            assert!(success, "Process failed unexpectedly");
        }
        x => panic!("Unexpected response: {:?}", x),
    };

    // Verify that we received nothing on stderr channel
    assert!(
        stderr.try_recv().is_err(),
        "Unexpectedly got result on stderr channel"
    );
}

#[rstest]
fn should_support_json_to_forward_stdin_to_remote_process(ctx: &'_ DistantServerCtx) {
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: SCRIPT_RUNNER.to_string(),
            args: vec![ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap().to_string()],
        }],
    };

    // distant action --format json --interactive
    let mut child = distant_subcommand(ctx, "action")
        .args(&["--format", "json"])
        .arg("--interactive")
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = spawn_line_reader(child.stdout.take().unwrap());
    let stderr = spawn_line_reader(child.stderr.take().unwrap());

    // Send our request as json
    let req_string = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin.write_all(req_string.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Get the indicator of a process started (first line returned can take ~7 seconds due to the
    // handshake cost)
    let out =
        friendly_recv_line(&stdout, Duration::from_secs(30)).expect("Failed to get proc start");
    let res: Response = serde_json::from_str(&out).unwrap();
    let id = match &res.payload[0] {
        ResponseData::ProcStart { id } => *id,
        x => panic!("Unexpected response: {:?}", x),
    };

    // Send stdin to remote process
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcStdin {
            id,
            data: String::from("hello world\n"),
        }],
    };
    let req_string = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin.write_all(req_string.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Should receive ok message & stdout message, although these may be in different order
    let (res1, res2) = next_two_msgs!(&stdout);
    match (&res1.payload[0], &res2.payload[0]) {
        (ResponseData::Ok, ResponseData::ProcStdout { data, .. }) => {
            assert_eq!(data, "hello world\n")
        }
        (ResponseData::ProcStdout { data, .. }, ResponseData::Ok) => {
            assert_eq!(data, "hello world\n")
        }
        x => panic!("Unexpected responses: {:?}", x),
    };

    // Kill the remote process since it only terminates when stdin closes, but we
    // want to verify that we get a proc done is some manner, which won't happen
    // if stdin closes as our interactive process will also close
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcKill { id }],
    };
    let req_string = format!("{}\n", serde_json::to_string(&req).unwrap());
    stdin.write_all(req_string.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Should receive ok message & process completion
    let (res1, res2) = next_two_msgs!(&stdout);
    match (&res1.payload[0], &res2.payload[0]) {
        (ResponseData::Ok, ResponseData::ProcDone { success, .. }) => {
            assert!(!success, "Process succeeded unexpectedly");
        }
        (ResponseData::ProcDone { success, .. }, ResponseData::Ok) => {
            assert!(!success, "Process succeeded unexpectedly");
        }
        x => panic!("Unexpected responses: {:?}", x),
    };

    // Verify that we received nothing on stderr channel
    assert!(
        stderr.try_recv().is_err(),
        "Unexpectedly got result on stderr channel"
    );
}

#[rstest]
fn should_support_json_output_for_error(mut action_cmd: Command) {
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
            args: Vec::new(),
        }],
    };

    // distant action --format json --interactive
    let cmd = action_cmd
        .args(&["--format", "json"])
        .arg("--interactive")
        .write_stdin(format!("{}\n", serde_json::to_string(&req).unwrap()))
        .assert()
        .success()
        .stderr("");

    let res: Response = serde_json::from_slice(&cmd.get_output().stdout).unwrap();
    assert!(
        matches!(
            res.payload[0],
            ResponseData::Error(Error {
                kind: ErrorKind::NotFound,
                ..
            })
        ),
        "Unexpected response: {:?}",
        res.payload[0]
    );
}
