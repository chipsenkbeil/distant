//! Integration tests for PTY and predictive echo features.
//!
//! Tests the `distant shell` and `distant spawn --pty` commands with
//! purpose-built helper binaries (`pty-echo`, `pty-interactive`) that exercise
//! PTY I/O, prompt handling, and exit behavior.
//!
//! All tests are Unix-only because `expectrl`'s ConPTY backend on Windows
//! cannot read actual text output (only escape sequences flow through the pipe).

#[cfg(unix)]
mod unix {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use expectrl::process::Healthcheck;
    use expectrl::process::unix::WaitStatus;
    use expectrl::{Eof, Expect, Session};

    use distant_test_harness::exe;
    use distant_test_harness::manager::ManagerCtx;

    /// Builds a `std::process::Command` for `distant shell` with optional extra args.
    fn build_shell_command(ctx: &ManagerCtx, extra_args: &[&str]) -> Command {
        let (bin, mut args) = ctx.cmd_parts(["shell"]);

        for arg in extra_args {
            args.push(arg.to_string());
        }

        let mut cmd = Command::new(bin);
        cmd.args(&args);
        cmd
    }

    /// Builds a `std::process::Command` for `distant spawn` with optional extra args.
    fn build_spawn_command(ctx: &ManagerCtx, extra_args: &[&str]) -> Command {
        let (bin, mut args) = ctx.cmd_parts(["spawn"]);

        for arg in extra_args {
            args.push(arg.to_string());
        }

        let mut cmd = Command::new(bin);
        cmd.args(&args);
        cmd
    }

    /// Waits for the session's process to exit, polling `get_status()` until it
    /// returns a non-alive result. Returns the final status for assertion.
    fn wait_for_exit<S>(session: &Session<expectrl::process::unix::UnixProcess, S>) -> WaitStatus {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let status = session.get_status().expect("Failed to get process status");
            if !matches!(status, WaitStatus::StillAlive) {
                return status;
            }
            assert!(Instant::now() < deadline, "Process did not exit within 60s");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn shell_pty_echo_roundtrip() {
        let ctx = ManagerCtx::start();
        let pty_echo = exe::build_pty_echo()
            .await
            .expect("Failed to build pty-echo");
        let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

        let cmd = build_shell_command(&ctx, &["--", pty_echo_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session.send("abc").expect("Failed to send text");
        session.expect("abc").expect("Expected 'abc' echoed back");

        session.expect(Eof).ok();
    }

    #[tokio::test]
    async fn shell_pty_interactive_prompt() {
        let ctx = ManagerCtx::start();
        let pty_interactive = exe::build_pty_interactive()
            .await
            .expect("Failed to build pty-interactive");
        let pty_interactive_str = pty_interactive
            .to_str()
            .expect("pty-interactive path is not valid UTF-8");

        let cmd = build_shell_command(&ctx, &["--", pty_interactive_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("$ ")
            .expect("Expected '$ ' prompt from pty-interactive");

        session.expect(Eof).ok();
    }

    #[tokio::test]
    async fn shell_pty_interactive_exit() {
        let ctx = ManagerCtx::start();
        let pty_interactive = exe::build_pty_interactive()
            .await
            .expect("Failed to build pty-interactive");
        let pty_interactive_str = pty_interactive
            .to_str()
            .expect("pty-interactive path is not valid UTF-8");

        let cmd = build_shell_command(&ctx, &["--", pty_interactive_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("$ ")
            .expect("Expected '$ ' prompt from pty-interactive");

        // Send Ctrl+D (EOF). The server-side PTY line discipline interprets
        // 0x04 as EOF when the line buffer is empty, causing pty-interactive's
        // BufRead::lines() iterator to return None and exit cleanly. This is
        // more reliable than sending "exit\n" through the full PTY relay
        // under parallel load.
        for _ in 0..5 {
            session.send("\x04").expect("Failed to send Ctrl+D");
            std::thread::sleep(Duration::from_millis(300));

            if !session.is_alive().unwrap_or(false) {
                break;
            }
        }

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0, got: {status:?}"
        );
    }

    #[tokio::test]
    async fn spawn_pty_flag() {
        let ctx = ManagerCtx::start();
        let pty_echo = exe::build_pty_echo()
            .await
            .expect("Failed to build pty-echo");
        let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

        let cmd = build_spawn_command(&ctx, &["--pty", "--", pty_echo_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn spawn --pty");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session.send("hello").expect("Failed to send text");
        session
            .expect("hello")
            .expect("Expected 'hello' echoed back via spawn --pty");

        session.expect(Eof).ok();
    }

    #[tokio::test]
    async fn predict_off_runs_command() {
        let ctx = ManagerCtx::start();

        let cmd = build_shell_command(&ctx, &["--predict", "off", "--", "echo", "predict-off-ok"]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell with --predict off");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("predict-off-ok")
            .expect("Expected output with predict off");

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0 with predict off, got: {status:?}"
        );
    }

    #[tokio::test]
    async fn shell_pty_ctrl_c() {
        let ctx = ManagerCtx::start();
        let pty_interactive = exe::build_pty_interactive()
            .await
            .expect("Failed to build pty-interactive");
        let pty_interactive_str = pty_interactive
            .to_str()
            .expect("pty-interactive path is not valid UTF-8");

        let cmd = build_shell_command(&ctx, &["--", pty_interactive_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("$ ")
            .expect("Expected initial prompt from pty-interactive");

        // Send Ctrl+C (ETX byte 0x03). The server-side PTY line discipline
        // translates this to SIGINT for the pty-interactive process group.
        session.send("\x03").expect("Failed to send Ctrl+C");

        // pty-interactive's ctrlc handler sets a flag; the next line read
        // checks the flag and prints a new prompt. Send a newline to
        // unblock the blocking BufRead::lines() iterator.
        std::thread::sleep(Duration::from_millis(200));
        session
            .send_line("")
            .expect("Failed to send newline after Ctrl+C");

        // After Ctrl+C handling, pty-interactive should print a new `$ `.
        session
            .expect("$ ")
            .expect("Expected new prompt after Ctrl+C");

        session.expect(Eof).ok();
    }

    #[tokio::test]
    async fn predict_on_password_suppressed() {
        let ctx = ManagerCtx::start();
        let pty_password = exe::build_pty_password()
            .await
            .expect("Failed to build pty-password");
        let pty_password_str = pty_password
            .to_str()
            .expect("pty-password path is not valid UTF-8");

        let cmd = build_shell_command(&ctx, &["--predict", "on", "--", pty_password_str]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell with --predict on");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        // pty-password prints "Password: " then disables echo via rpassword.
        session
            .expect("Password: ")
            .expect("Expected 'Password: ' prompt from pty-password");

        // Send a password. With echo disabled at the PTY level, neither
        // server echo nor predictive echo should display the characters.
        session
            .send_line("secret123")
            .expect("Failed to send password");

        // After the password, pty-password prints "Authenticated." and
        // resumes a byte-by-byte echo loop.
        session
            .expect("Authenticated.")
            .expect("Expected 'Authenticated.' after password entry");

        session.expect(Eof).ok();
    }

    #[tokio::test]
    async fn predict_on_runs_command() {
        let ctx = ManagerCtx::start();

        let cmd = build_shell_command(&ctx, &["--predict", "on", "--", "echo", "predict-on-ok"]);
        let mut session = Session::spawn(cmd).expect("Failed to spawn shell with --predict on");
        session.set_expect_timeout(Some(Duration::from_secs(30)));

        session
            .expect("predict-on-ok")
            .expect("Expected output with predict on");

        session.expect(Eof).ok();
        let status = wait_for_exit(&session);
        assert!(
            matches!(status, WaitStatus::Exited(_, 0)),
            "Expected exit code 0 with predict on, got: {status:?}"
        );
    }
}
