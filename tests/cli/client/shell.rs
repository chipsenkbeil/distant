//! Integration tests for the `distant shell` CLI subcommand.
//!
//! Uses `expectrl` to spawn the shell process inside a real PTY, which is
//! required because `distant shell` uses `termwiz::terminal::new_terminal()`
//! and needs stdin/stdout to be a TTY.

use std::process::Command;
use std::time::{Duration, Instant};

use expectrl::Session;
use expectrl::process::Healthcheck;
use expectrl::process::unix::WaitStatus;
use expectrl::{Eof, Expect};
use rstest::*;

use distant_test_harness::manager::*;

/// Builds a `std::process::Command` from ManagerCtx for use with `Session::spawn`.
///
/// We use `Session::spawn(Command)` rather than `expectrl::spawn(string)` because
/// the string-based API uses a regex tokenizer that doesn't strip shell quotes,
/// causing arguments with spaces or special characters to be mangled.
fn build_shell_command(ctx: &ManagerCtx, extra_args: &[&str]) -> Command {
    let (bin, mut args) = ctx.cmd_parts(["shell"]);

    for arg in extra_args {
        args.push(arg.to_string());
    }

    let mut cmd = Command::new(bin);
    cmd.args(&args);
    cmd
}

/// Waits for the session's process to exit, polling `get_status()` until it
/// returns an `Exited` status. `expectrl`'s `get_status()` uses non-blocking
/// `waitpid`, so it may return `StillAlive` briefly after EOF is received.
fn wait_for_exit<P, S>(session: &Session<P, S>) -> WaitStatus
where
    P: Healthcheck<Status = WaitStatus>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = session.get_status().expect("Failed to get process status");
        if !matches!(status, WaitStatus::StillAlive) {
            return status;
        }
        assert!(Instant::now() < deadline, "Process did not exit within 30s");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[rstest]
#[test_log::test]
fn should_run_single_command_via_shell(ctx: ManagerCtx) {
    let echo_args: Vec<&str> = if cfg!(windows) {
        vec!["--", "cmd.exe", "/c", "echo", "hello"]
    } else {
        vec!["--", "echo", "hello"]
    };

    let cmd = build_shell_command(&ctx, &echo_args);
    let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
    session.set_expect_timeout(Some(Duration::from_secs(30)));

    session.expect("hello").expect("Expected 'hello' in output");

    // Wait for process to finish
    session.expect(Eof).ok();
    let status = wait_for_exit(&session);
    assert!(
        matches!(status, WaitStatus::Exited(_, 0)),
        "Expected exit code 0, got: {status:?}"
    );
}

#[rstest]
#[test_log::test]
fn should_forward_exit_code(ctx: ManagerCtx) {
    // Note: distant shell joins CMD args with spaces (`cmd.join(" ")`), so
    // multi-word `-c` arguments like `bash -c "exit 42"` lose their grouping.
    // Use `false` (exit code 1) to test non-zero exit code forwarding.
    let exit_args: Vec<&str> = if cfg!(windows) {
        vec!["--", "cmd.exe", "/c", "exit", "1"]
    } else {
        vec!["--", "false"]
    };

    let cmd = build_shell_command(&ctx, &exit_args);
    let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
    session.set_expect_timeout(Some(Duration::from_secs(30)));

    // Wait for process to finish
    session.expect(Eof).ok();
    let status = wait_for_exit(&session);
    assert!(
        matches!(status, WaitStatus::Exited(_, 1)),
        "Expected exit code 1, got: {status:?}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_current_dir(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let temp_str = temp.path().to_str().unwrap();

    let pwd_args: Vec<&str> = if cfg!(windows) {
        vec!["--current-dir", temp_str, "--", "cmd.exe", "/c", "cd"]
    } else {
        vec!["--current-dir", temp_str, "--", "pwd"]
    };

    let cmd = build_shell_command(&ctx, &pwd_args);
    let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
    session.set_expect_timeout(Some(Duration::from_secs(30)));

    // The output should contain the temp directory path (possibly canonicalized)
    let canonical = temp.path().canonicalize().unwrap();
    let expected = canonical.to_str().unwrap();
    session
        .expect(expected)
        .unwrap_or_else(|_| panic!("Expected output to contain '{expected}'"));

    // Wait for process to finish
    session.expect(Eof).ok();
    let status = wait_for_exit(&session);
    assert!(
        matches!(status, WaitStatus::Exited(_, 0)),
        "Expected exit code 0, got: {status:?}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_environment(ctx: ManagerCtx) {
    // Use `env` (or `set` on Windows) to list all environment variables, then
    // search for our custom variable. This is more reliable than `printenv`
    // through a PTY since `env` output is longer and gives the stdout task
    // more time to flush before the process exits.
    let env_args: Vec<&str> = if cfg!(windows) {
        vec!["--environment", "FOO=\"bar\"", "--", "cmd.exe", "/c", "set"]
    } else {
        vec!["--environment", "FOO=\"bar\"", "--", "env"]
    };

    let cmd = build_shell_command(&ctx, &env_args);
    let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
    session.set_expect_timeout(Some(Duration::from_secs(30)));

    session
        .expect("FOO=bar")
        .expect("Expected 'FOO=bar' in output");

    // Wait for process to finish
    session.expect(Eof).ok();
    let status = wait_for_exit(&session);
    assert!(
        matches!(status, WaitStatus::Exited(_, 0)),
        "Expected exit code 0, got: {status:?}"
    );
}
