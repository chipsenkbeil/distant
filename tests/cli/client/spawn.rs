//! Integration tests for the `distant spawn` CLI subcommand.
//!
//! Tests executing remote processes, capturing stdout/stderr, forwarding stdin,
//! exit code forwarding, environment variables, PTY support, and error handling.

use std::io::Write;

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::pty::PtySession;
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

    let mut session = PtySession::spawn(&bin, &args);
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
#[test_log::test]
fn should_fail_for_nonexistent_command(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "nonexistent-command-xyz"])
        .output()
        .expect("Failed to run spawn");

    assert!(
        !output.status.success(),
        "spawn of nonexistent command should fail"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_current_dir(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("spawn-cwd");
    ctx.cli_mkdir(&dir);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--current-dir", &dir, "--", "pwd"])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn --current-dir should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dir_basename = dir.rsplit('/').next().unwrap_or(&dir);
    assert!(
        stdout.contains(dir_basename),
        "Expected working directory to contain '{dir_basename}', got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_shell_flag(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--shell", "--", "echo", "shell-flag-ok"])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn --shell should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("shell-flag-ok"),
        "Expected 'shell-flag-ok' in stdout, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_capture_stderr(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    // Docker always targets Unix containers; Host and SSH target the local OS
    let shell_cmd = if cfg!(windows) && matches!(backend, Backend::Host | Backend::Ssh) {
        "cmd /c echo error_msg 1>&2"
    } else {
        "sh -c 'echo error_msg >&2'"
    };

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["-c", shell_cmd])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error_msg"),
        "Expected 'error_msg' in stderr, got: {stderr}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_forward_stdin(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    // head -1 reads exactly one line from stdin and exits.
    // On Windows (Host/SSH), findstr "^" matches all lines — pipe one line + close stdin.
    let args: Vec<&str> = if cfg!(windows) && matches!(backend, Backend::Host | Backend::Ssh) {
        vec!["--", "findstr", "^"]
    } else {
        vec!["--", "head", "-1"]
    };

    let mut child = ctx
        .new_std_cmd(["spawn"])
        .args(&args)
        .spawn()
        .expect("Failed to spawn");

    let mut stdin = child.stdin.take().expect("stdin should be piped");
    stdin
        .write_all(b"stdin-forwarded\n")
        .expect("Failed to write to stdin");
    drop(stdin);

    let output = child
        .wait_with_output()
        .expect("Failed to wait for child process");

    assert!(
        output.status.success(),
        "spawn with stdin should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("stdin-forwarded"),
        "Expected 'stdin-forwarded' in stdout, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_forward_exit_code(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let shell_cmd = if cfg!(windows) && matches!(backend, Backend::Host | Backend::Ssh) {
        "cmd /c exit /b 42"
    } else {
        "sh -c 'exit 42'"
    };

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["-c", shell_cmd])
        .output()
        .expect("Failed to run spawn");

    assert!(
        !output.status.success(),
        "spawn of 'exit 42' should fail (non-zero exit)"
    );

    let code = output.status.code().expect("process should have exit code");
    assert_eq!(code, 42, "Expected exit code 42, got: {code}");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_forward_environment_variables(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let shell_cmd = if cfg!(windows) && matches!(backend, Backend::Host | Backend::Ssh) {
        "cmd /c echo %DISTANT_TEST_VAR%"
    } else {
        "sh -c 'echo $DISTANT_TEST_VAR'"
    };

    let output = ctx
        .new_std_cmd(["spawn"])
        .args([
            "--environment",
            "DISTANT_TEST_VAR=hello_from_env",
            "-c",
            shell_cmd,
        ])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn with --environment should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello_from_env"),
        "Expected 'hello_from_env' in stdout, got: {stdout}"
    );
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
    let mut session = PtySession::spawn(&bin, &args);

    session.resize(50, 132);
    session.expect("50");
}
