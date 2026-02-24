//! Integration tests for the `distant spawn` CLI subcommand.
//!
//! Tests executing remote processes, capturing stdout/stderr, forwarding stdin,
//! exit code reflection, and error handling for non-existent binaries.

use rstest::*;

use distant_test_harness::manager::*;
use distant_test_harness::scripts::*;
use distant_test_harness::utils::regex_pred;

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

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_support_dash_c_flag(ctx: ManagerCtx) {
    // distant spawn -c "echo hello"
    ctx.cmd("spawn")
        .args(["-c", "echo hello"])
        .assert()
        .success()
        .stdout("hello\n");
}

#[cfg(windows)]
#[rstest]
#[test_log::test]
fn should_support_dash_c_flag(ctx: ManagerCtx) {
    // distant spawn -c "echo hello"
    ctx.cmd("spawn")
        .args(["-c", "echo hello"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hello"));
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_support_current_dir_option(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // distant spawn --current-dir {path} -- pwd
    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--current-dir"])
        .arg(temp.path())
        .args(["--", "pwd"])
        .output()
        .expect("Failed to run spawn");

    assert!(output.status.success(), "spawn should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let canonical = temp.path().canonicalize().unwrap();
    assert!(
        stdout.trim() == canonical.to_str().unwrap(),
        "Expected current-dir to be {canonical:?}, got: {stdout}"
    );
}

#[cfg(windows)]
#[rstest]
#[test_log::test]
fn should_support_current_dir_option(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    // distant spawn --current-dir {path} -- cmd /c cd
    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--current-dir"])
        .arg(temp.path())
        .args(["--", "cmd", "/c", "cd"])
        .output()
        .expect("Failed to run spawn");

    assert!(output.status.success(), "spawn should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_trimmed = stdout.trim();
    // On Windows, canonicalize() returns \\?\ prefix and `cmd /c cd` may return short names,
    // so just verify the output is a non-empty absolute path (the command ran in the right dir)
    assert!(
        !stdout_trimmed.is_empty() && std::path::Path::new(stdout_trimmed).is_absolute(),
        "Expected absolute path from current-dir, got: {stdout}"
    );
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_support_environment_option(ctx: ManagerCtx) {
    // distant spawn --environment 'MY_TEST_VAR=hello_from_distant' -- printenv MY_TEST_VAR
    ctx.new_assert_cmd(["spawn"])
        .args([
            "--environment",
            "MY_TEST_VAR=\"hello_from_distant\"",
            "--",
            "printenv",
            "MY_TEST_VAR",
        ])
        .assert()
        .success()
        .stdout("hello_from_distant\n");
}

#[cfg(windows)]
#[rstest]
#[test_log::test]
fn should_support_environment_option(ctx: ManagerCtx) {
    // distant spawn --environment 'MY_TEST_VAR=hello_from_distant' -- cmd /c echo %MY_TEST_VAR%
    let output = ctx
        .new_assert_cmd(["spawn"])
        .args([
            "--environment",
            "MY_TEST_VAR=\"hello_from_distant\"",
            "--",
            "cmd",
            "/c",
            "echo",
            "%MY_TEST_VAR%",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("hello_from_distant"),
        "Expected env var to be expanded, got: {stdout}"
    );
}

#[cfg(unix)]
#[rstest]
#[test_log::test]
fn should_support_shell_flag(ctx: ManagerCtx) {
    // distant spawn --shell -- 'echo $HOME'
    // When --shell is used, the command is wrapped in a shell, allowing variable expansion
    let output = ctx
        .new_assert_cmd(["spawn"])
        .args(["--shell", "--", "echo $HOME"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // $HOME should have been expanded by the shell (not printed literally)
    assert!(
        !stdout.contains("$HOME"),
        "Expected shell to expand $HOME, got literal: {stdout}"
    );
    assert!(
        !stdout.trim().is_empty(),
        "Expected non-empty output from shell expansion"
    );
}

#[cfg(windows)]
#[rstest]
#[test_log::test]
fn should_support_shell_flag(ctx: ManagerCtx) {
    // distant spawn --shell -- 'echo %USERPROFILE%'
    // When --shell is used, the command is wrapped in a shell, allowing variable expansion
    let output = ctx
        .new_assert_cmd(["spawn"])
        .args(["--shell", "--", "echo %USERPROFILE%"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // %USERPROFILE% should have been expanded by the shell (not printed literally)
    assert!(
        !stdout.contains("%USERPROFILE%"),
        "Expected shell to expand %USERPROFILE%, got literal: {stdout}"
    );
    assert!(
        !stdout.trim().is_empty(),
        "Expected non-empty output from shell expansion"
    );
}
