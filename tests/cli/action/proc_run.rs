use crate::cli::{
    fixtures::*,
    utils::{random_tenant, regex_pred, FAILURE_LINE},
};
use assert_cmd::Command;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;
use std::path::PathBuf;

lazy_static::lazy_static! {
    static ref SCRIPT_DIR: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts").join("test");

    static ref ECHO_ARGS_TO_STDOUT_SH: PathBuf = SCRIPT_DIR.join("echo_args_to_stdout.sh");
    static ref ECHO_ARGS_TO_STDERR_SH: PathBuf = SCRIPT_DIR.join("echo_args_to_stderr.sh");
    static ref ECHO_STDIN_TO_STDOUT_SH: PathBuf = SCRIPT_DIR.join("echo_stdin_to_stdout.sh");
    static ref EXIT_CODE_SH: PathBuf = SCRIPT_DIR.join("exit_code.sh");

    static ref DOES_NOT_EXIST_BIN: PathBuf = SCRIPT_DIR.join("does_not_exist_bin");
}

#[rstest]
fn should_execute_program_and_return_exit_status(mut action_cmd: Command) {
    // distant action proc-run -- {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
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
        .arg(ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap())
        .write_stdin("hello world\n")
        .assert()
        .success()
        .stdout("hello world\n")
        .stderr("");
}

#[rstest]
fn yield_an_error_when_fails(mut action_cmd: Command) {
    // distant action proc-run {cmd} [args]
    action_cmd
        .args(&["proc-run", "--"])
        .arg(EXIT_CODE_SH.to_str().unwrap())
        .arg("3")
        .assert()
        .code(3)
        .stdout("")
        .stderr("");
}

#[rstest]
fn should_support_json_to_execute_program_and_return_exit_status(mut action_cmd: Command) {
    let req = Request {
        id: rand::random(),
        tenant: random_tenant(),
        payload: vec![RequestData::ProcRun {
            cmd: ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string(),
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
        matches!(res.payload[0], ResponseData::ProcStart { .. }),
        "Unexpected response: {:?}",
        res.payload[0],
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
