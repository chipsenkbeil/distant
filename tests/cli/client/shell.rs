//! Integration tests for the `distant shell` CLI subcommand.
//!
//! Uses `expectrl` to spawn the shell process inside a real PTY, which is
//! required because `distant shell` uses `termwiz::terminal::new_terminal()`
//! and needs stdin/stdout to be a TTY.
//!
//! On Windows, `expectrl`'s ConPTY `expect()` cannot read actual text output
//! (only escape sequences flow through the pipe), so we use file-based
//! verification: redirect command output to a temp file, wait for exit, then
//! check the file contents.

use std::process::Command;
use std::time::{Duration, Instant};

use expectrl::Session;
use expectrl::process::Healthcheck;
#[cfg(unix)]
use expectrl::process::unix::WaitStatus;
#[cfg(unix)]
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
/// returns a non-alive result. Returns the final status for assertion.
#[cfg(unix)]
fn wait_for_exit<S>(session: &Session<expectrl::process::unix::UnixProcess, S>) -> WaitStatus {
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

/// Waits for the session's process to exit by polling `is_alive()`.
/// On Windows, `expectrl` doesn't expose exit codes via `get_status()`,
/// so we just poll `is_alive()` and don't return a status.
#[cfg(windows)]
fn wait_for_exit<P, S>(session: &Session<P, S>)
where
    P: Healthcheck,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if !session.is_alive().expect("Failed to check process status") {
            return;
        }
        assert!(Instant::now() < deadline, "Process did not exit within 30s");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[rstest]
#[test_log::test]
fn should_run_single_command_via_shell(ctx: ManagerCtx) {
    #[cfg(unix)]
    {
        let cmd = build_shell_command(&ctx, &["--", "echo", "hello"]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session.expect("hello").expect("Expected 'hello' in output");

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0, got: {status:?}"
        );
    }

    #[cfg(windows)]
    {
        let temp = assert_fs::TempDir::new().unwrap();
        let marker = temp.path().join("output.txt");
        let marker_str = marker.to_str().unwrap();

        let args = vec!["--", "cmd.exe", "/c", "echo", "hello", ">", marker_str];
        let cmd = build_shell_command(&ctx, &args);
        let session = Session::spawn(cmd).expect("Failed to spawn shell");
        wait_for_exit(&session);

        let contents = std::fs::read_to_string(&marker)
            .unwrap_or_else(|e| panic!("Failed to read marker file {marker_str}: {e}"));
        assert!(
            contents.contains("hello"),
            "Expected 'hello' in output file, got: {contents:?}"
        );
    }
}

#[rstest]
#[test_log::test]
fn should_forward_exit_code(ctx: ManagerCtx) {
    #[cfg(unix)]
    {
        let cmd = build_shell_command(&ctx, &["--", "false"]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 1)),
            "Expected exit code 1, got: {status:?}"
        );
    }

    #[cfg(windows)]
    {
        // Verify that `distant shell -- cmd.exe /c exit 1` terminates.
        // On Windows, expectrl doesn't expose the exit code, but we verify
        // the process ran and exited (non-hanging).
        let args = vec!["--", "cmd.exe", "/c", "exit", "1"];
        let cmd = build_shell_command(&ctx, &args);
        let session = Session::spawn(cmd).expect("Failed to spawn shell");
        wait_for_exit(&session);
        // If we get here without timeout, the process exited successfully
    }
}

#[rstest]
#[test_log::test]
fn should_support_current_dir(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let temp_str = temp.path().to_str().unwrap();

    #[cfg(unix)]
    {
        let pwd_args = vec!["--current-dir", temp_str, "--", "pwd"];
        let cmd = build_shell_command(&ctx, &pwd_args);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        let canonical = temp.path().canonicalize().unwrap();
        let expected = canonical.to_str().unwrap();
        session
            .expect(expected)
            .unwrap_or_else(|_| panic!("Expected output to contain '{expected}'"));

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0, got: {status:?}"
        );
    }

    #[cfg(windows)]
    {
        let marker = temp.path().join("cwd_output.txt");
        let marker_str = marker.to_str().unwrap();

        let args = vec![
            "--current-dir",
            temp_str,
            "--",
            "cmd.exe",
            "/c",
            "cd",
            ">",
            marker_str,
        ];
        let cmd = build_shell_command(&ctx, &args);
        let session = Session::spawn(cmd).expect("Failed to spawn shell");
        wait_for_exit(&session);

        let contents = std::fs::read_to_string(&marker)
            .unwrap_or_else(|e| panic!("Failed to read marker file {marker_str}: {e}"));
        let contents_path =
            distant_test_harness::utils::normalize_path(std::path::Path::new(contents.trim()));
        let contents_str = contents_path.to_string_lossy();
        let canonical = temp.path().canonicalize().unwrap();
        let canonical_str = canonical.to_str().unwrap();
        let expected = canonical_str.strip_prefix(r"\\?\").unwrap_or(canonical_str);
        assert!(
            contents_str.contains(expected),
            "Expected output to contain '{expected}', got: {contents_str:?}"
        );
    }
}

#[rstest]
#[test_log::test]
fn should_support_environment(ctx: ManagerCtx) {
    #[cfg(unix)]
    {
        let env_args = vec!["--environment", "FOO=\"bar\"", "--", "env"];
        let cmd = build_shell_command(&ctx, &env_args);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("FOO=bar")
            .expect("Expected 'FOO=bar' in output");

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0, got: {status:?}"
        );
    }

    #[cfg(windows)]
    {
        let temp = assert_fs::TempDir::new().unwrap();
        let marker = temp.path().join("env_output.txt");
        let marker_str = marker.to_str().unwrap();

        let args = vec![
            "--environment",
            "FOO=\"bar\"",
            "--",
            "cmd.exe",
            "/c",
            "set",
            ">",
            marker_str,
        ];
        let cmd = build_shell_command(&ctx, &args);
        let session = Session::spawn(cmd).expect("Failed to spawn shell");
        wait_for_exit(&session);

        let contents = std::fs::read_to_string(&marker)
            .unwrap_or_else(|e| panic!("Failed to read marker file {marker_str}: {e}"));
        assert!(
            contents.contains("FOO=bar"),
            "Expected 'FOO=bar' in output file, got: {contents:?}"
        );
    }
}
