//! Integration tests for the `distant shell` CLI subcommand.
//!
//! Uses `portable-pty` for cross-platform PTY session management via
//! [`PtySession`](crate::cli::pty::PtySession). Tests shell execution,
//! exit code forwarding, prediction modes, and alternate screen handling.

use std::path::PathBuf;
use std::time::Duration;

use rstest::*;

use distant_test_harness::backend::{Backend, BackendCtx};
use distant_test_harness::skip_if_no_backend;

fn shell_cmd_args(ctx: &BackendCtx, extra_args: &[&str]) -> (PathBuf, Vec<String>) {
    let (bin, mut args) = ctx.cmd_parts(["shell"]);
    for arg in extra_args {
        args.push(arg.to_string());
    }
    (bin, args)
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_run_individual_command(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    #[cfg(unix)]
    let extra_args: &[&str] = &["--", "echo", "hello"];
    #[cfg(windows)]
    let extra_args: &[&str] = &["--", "cmd", "/c", "echo", "hello"];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("hello");
    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_echo_input_through_pty(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_echo_str = ctx
        .prepare_binary("pty-echo")
        .await
        .expect("Failed to build pty-echo");

    let (bin, args) = shell_cmd_args(&ctx, &["--", &pty_echo_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.send("abc");
    session.expect("abc");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_display_interactive_prompt(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_interactive_str = ctx
        .prepare_binary("pty-interactive")
        .await
        .expect("Failed to build pty-interactive");

    let (bin, args) = shell_cmd_args(&ctx, &["--", &pty_interactive_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("$ ");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_exit_on_eof_signal(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_interactive_str = ctx
        .prepare_binary("pty-interactive")
        .await
        .expect("Failed to build pty-interactive");

    let (bin, args) = shell_cmd_args(&ctx, &["--", &pty_interactive_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("$ ");

    // On Unix, Ctrl-D sends EOF to the PTY which closes the shell cleanly.
    #[cfg(unix)]
    for _ in 0..5 {
        session.send("\x04");
        std::thread::sleep(Duration::from_millis(300));
        if !session.is_alive() {
            break;
        }
    }

    // On Windows, we send "exit" because ConPTY doesn't support Ctrl-D (EOF signal).
    #[cfg(windows)]
    session.send_line("exit");

    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_run_command_with_predict_off(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    #[cfg(unix)]
    let extra_args: &[&str] = &["--predict", "off", "--", "echo", "predict-off-ok"];
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "cmd",
        "/c",
        "echo",
        "predict-off-ok",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("predict-off-ok");
    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0 with predict off");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_handle_ctrl_c_interrupt(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_interactive_str = ctx
        .prepare_binary("pty-interactive")
        .await
        .expect("Failed to build pty-interactive");

    let (bin, args) = shell_cmd_args(&ctx, &["--", &pty_interactive_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("$ ");
    session.send("\x03");
    std::thread::sleep(Duration::from_millis(200));
    session.send_line("");
    session.expect("$ ");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_suppress_predicted_password_echo(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_password_str = ctx
        .prepare_binary("pty-password")
        .await
        .expect("Failed to build pty-password");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", &pty_password_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_run_command_with_predict_on(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    #[cfg(unix)]
    let extra_args: &[&str] = &["--predict", "on", "--", "echo", "predict-on-ok"];
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "on",
        "--",
        "cmd",
        "/c",
        "echo",
        "predict-on-ok",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("predict-on-ok");
    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0 with predict on");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_not_echo_locally_with_predict_off(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_password_str = ctx
        .prepare_binary("pty-password")
        .await
        .expect("Failed to build pty-password");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "off", "--", &pty_password_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_echo_from_server_only_with_predict_off(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_echo_str = ctx
        .prepare_binary("pty-echo")
        .await
        .expect("Failed to build pty-echo");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "off", "--", &pty_echo_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);
    session.set_timeout(Duration::from_secs(60));

    session.expect("Connected to manager");

    session.send("x");
    session.expect("x");
    session.send("y");
    session.expect("y");
    session.send("z");
    session.expect("z");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_echo_immediately_with_predict_on(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_echo_str = ctx
        .prepare_binary("pty-echo")
        .await
        .expect("Failed to build pty-echo");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", &pty_echo_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.send("predict-immediate");
    session.expect("predict-immediate");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_correct_prediction_mismatch(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let pty_password_str = ctx
        .prepare_binary("pty-password")
        .await
        .expect("Failed to build pty-password");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", &pty_password_str]);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_enter_alternate_screen(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    #[cfg(unix)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "sh",
        "-c",
        "'tput smcup 2>/dev/null; tput rmcup 2>/dev/null; echo ALT_ENTRY_OK'",
    ];

    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "powershell",
        "-NoProfile",
        "-Command",
        "\"Write-Host -NoNewline ([char]27+'[?1049h'); Write-Host -NoNewline ([char]27+'[?1049l'); Write-Host 'ALT_ENTRY_OK'\"",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("ALT_ENTRY_OK");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_exit_alternate_screen(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    #[cfg(unix)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "sh",
        "-c",
        "'tput smcup 2>/dev/null; echo IN_ALT; tput rmcup 2>/dev/null; echo RESTORED'",
    ];

    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "powershell",
        "-NoProfile",
        "-Command",
        "\"Write-Host -NoNewline ([char]27+'[?1049h'); Write-Host 'IN_ALT'; Write-Host -NoNewline ([char]27+'[?1049l'); Write-Host 'RESTORED'\"",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = crate::cli::pty::PtySession::spawn(&bin, &args);

    session.expect("RESTORED");
}
