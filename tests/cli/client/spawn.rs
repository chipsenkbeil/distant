use rstest::*;

use crate::cli::scripts::*;
use crate::common::fixtures::*;
use crate::common::utils::regex_pred;

#[rstest]
#[test_log::test]
fn should_execute_program_and_return_exit_status(ctx: ManagerCtx) {
    // Windows prints out a message whereas unix prints nothing
    #[cfg(windows)]
    let stdout = regex_pred(".+");
    #[cfg(unix)]
    let stdout = "";

    // distant spawn -- {cmd} [args]
    ctx.cmd("spawn")
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(EXIT_CODE.to_str().unwrap())
        .arg("0")
        .assert()
        .success()
        .stdout(stdout);
}

#[rstest]
#[test_log::test]
fn should_capture_and_print_stdout(ctx: ManagerCtx) {
    // distant spawn -- {cmd} [args]
    ctx.cmd("spawn")
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(ECHO_ARGS_TO_STDOUT.to_str().unwrap())
        .arg("hello world")
        .assert()
        .success()
        .stdout(if cfg!(windows) {
            "hello world\r\n"
        } else {
            "hello world"
        });
}

#[rstest]
#[test_log::test]
fn should_capture_and_print_stderr(ctx: ManagerCtx) {
    // distant spawn -- {cmd} [args]
    ctx.cmd("spawn")
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(ECHO_ARGS_TO_STDERR.to_str().unwrap())
        .arg("hello world")
        .assert()
        .success()
        .stdout("")
        .stderr(predicates::str::contains("hello world"));
}

// TODO: This used to work fine with the assert_cmd where stdin would close from our
//       process, which would in turn lead to the remote process stdin being closed
//       and then the process exiting. This may be a bug we've introduced with the
//       refactor and should be revisited some day.
#[rstest]
#[test_log::test]
#[allow(clippy::zombie_processes)] // Test intentionally spawns child without waiting
fn should_forward_stdin_to_remote_process(ctx: ManagerCtx) {
    use std::io::{BufRead, BufReader, Write};

    // distant action proc-spawn {cmd} [args]
    let mut child = ctx
        .new_std_cmd(["spawn"])
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(ECHO_STDIN_TO_STDOUT.to_str().unwrap())
        .spawn()
        .expect("Failed to spawn process");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(if cfg!(windows) {
            b"hello world\r\n"
        } else {
            b"hello world\n"
        })
        .expect("Failed to write to stdin of process");

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).expect("Failed to read line");
    assert_eq!(
        line,
        if cfg!(windows) {
            "hello world\r\n"
        } else {
            "hello world\n"
        }
    );

    child.kill().expect("Failed to kill spawned process");
}

#[rstest]
#[test_log::test]
fn reflect_the_exit_code_of_the_process(ctx: ManagerCtx) {
    // Windows prints out a message whereas unix prints nothing
    #[cfg(windows)]
    let stdout = regex_pred(".+");
    #[cfg(unix)]
    let stdout = "";

    // distant spawn -- {cmd} [args]
    ctx.cmd("spawn")
        .arg("--")
        .arg(SCRIPT_RUNNER.as_str())
        .arg(SCRIPT_RUNNER_ARG.as_str())
        .arg(EXIT_CODE.to_str().unwrap())
        .arg("99")
        .assert()
        .code(99)
        .stdout(stdout);
}

#[rstest]
#[test_log::test]
fn yield_an_error_when_fails(ctx: ManagerCtx) {
    // distant spawn -- {cmd} [args]
    ctx.cmd("spawn")
        .arg("--")
        .arg(DOES_NOT_EXIST_BIN.to_str().unwrap())
        .assert()
        .code(1)
        .stdout("")
        .stderr(regex_pred(".+"));
}
