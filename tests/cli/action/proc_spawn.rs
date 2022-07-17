use crate::cli::{fixtures::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use assert_fs::prelude::*;
use distant::ExitCode;
use once_cell::sync::Lazy;
use rstest::*;
use std::process::Command as StdCommand;

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

static EXIT_CODE_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("exit_code.sh");
    script
        .write_str(indoc::indoc!(
            r#"
            #!/usr/bin/env bash
            exit "$1"
        "#
        ))
        .unwrap();
    script
});

static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

#[rstest]
fn should_execute_program_and_return_exit_status(mut action_cmd: Command) {
    // distant action proc-spawn -- {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
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
    // distant action proc-spawn {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
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
    // distant action proc-spawn {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(ECHO_ARGS_TO_STDERR_SH.to_str().unwrap())
        .arg("hello world")
        .assert()
        .success()
        .stdout("")
        .stderr("hello world");
}

// TODO: This used to work fine with the assert_cmd where stdin would close from our
//       process, which would in turn lead to the remote process stdin being closed
//       and then the process exiting. This may be a bug we've introduced with the
//       refactor and should be revisited some day.
#[rstest]
fn should_forward_stdin_to_remote_process(mut action_std_cmd: StdCommand) {
    use std::io::{BufRead, BufReader, Write};

    // distant action proc-spawn {cmd} [args]
    let mut child = action_std_cmd
        .args(&["proc-spawn", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap())
        .spawn()
        .expect("Failed to spawn process");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"hello world\n")
        .expect("Failed to write to stdin of process");

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).expect("Failed to read line");
    assert_eq!(line, "hello world\n");

    child.kill().expect("Failed to kill spawned process");
}

#[rstest]
fn reflect_the_exit_code_of_the_process(mut action_cmd: Command) {
    // distant action proc-spawn {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
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
    // distant action proc-spawn {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
        .arg(DOES_NOT_EXIST_BIN.to_str().unwrap())
        .assert()
        .code(ExitCode::IoError.to_i32())
        .stdout("")
        .stderr(FAILURE_LINE.clone());
}
