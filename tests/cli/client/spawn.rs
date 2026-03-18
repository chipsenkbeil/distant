//! Integration tests for the `distant spawn` CLI subcommand.
//!
//! Tests executing remote processes, capturing stdout/stderr, forwarding stdin,
//! exit code reflection, PTY support, and error handling.
//! Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_execute_and_capture_stdout(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "echo", "spawn-ok"])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("spawn-ok"),
        "Expected 'spawn-ok' in stdout, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_support_pty_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (bin, mut args) = ctx.cmd_parts(["spawn"]);
    args.push("--pty".to_string());
    args.push("--".to_string());

    #[cfg(windows)]
    {
        args.push("cmd".to_string());
        args.push("/c".to_string());
    }
    args.push("echo".to_string());
    args.push("pty-spawn-ok".to_string());

    let mut session = super::super::pty::PtySession::spawn(&bin, &args);
    session.expect("pty-spawn-ok");
}

#[cfg(unix)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_dash_c_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    ctx.new_assert_cmd(["spawn"])
        .args(["-c", "echo hello"])
        .assert()
        .success()
        .stdout("hello\n");
}

#[cfg(windows)]
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_dash_c_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    ctx.new_assert_cmd(["spawn"])
        .args(["-c", "echo hello"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hello"));
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[tokio::test]
async fn should_propagate_pty_resize(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let delay_str = "2";

    #[cfg(unix)]
    let sleep_cmd = format!("'sleep {delay_str}; stty size'");
    #[cfg(unix)]
    let extra_args: Vec<&str> = vec!["--predict", "off", "--", "sh", "-c", &sleep_cmd];

    #[cfg(windows)]
    let extra_args: Vec<&str> = vec![
        "--predict",
        "off",
        "--",
        "cmd",
        "/c",
        "timeout",
        "/t",
        delay_str,
        "/nobreak",
        ">nul",
        "2>nul",
        "&",
        "mode",
        "con",
    ];

    let (bin, mut args) = ctx.cmd_parts(["shell"]);
    for arg in &extra_args {
        args.push(arg.to_string());
    }
    let mut session = super::super::pty::PtySession::spawn(&bin, &args);

    session.resize(50, 132);
    session.expect("50");
}
