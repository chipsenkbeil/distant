use crate::cli::{fixtures::*, utils::random_tenant};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use distant_core::{
    data::{Error, ErrorKind},
    Request, RequestData, Response, ResponseData,
};
use rstest::*;

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
