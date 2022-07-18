use crate::cli::{fixtures::*, scripts::*, utils::FAILURE_LINE};
use assert_cmd::Command;
use distant::ExitCode;
use rstest::*;
use std::process::Command as StdCommand;

#[rstest]
fn should_execute_program_and_return_exit_status(mut action_cmd: Command) {
    // distant action proc-spawn -- {cmd} [args]
    action_cmd
        .args(&["proc-spawn", "--"])
        .arg(SCRIPT_RUNNER.as_str())
        .arg(EXIT_CODE.to_str().unwrap())
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
        .arg(ECHO_ARGS_TO_STDOUT.to_str().unwrap())
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
        .arg(ECHO_ARGS_TO_STDERR.to_str().unwrap())
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
        .arg(ECHO_STDIN_TO_STDOUT.to_str().unwrap())
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
        .arg(EXIT_CODE.to_str().unwrap())
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
