//! Integration tests for the `distant shell` CLI subcommand.
//!
//! The `shell` command requires a PTY (pseudo-terminal), which is not available
//! in CI/test environments. These tests are marked `#[ignore]` and must be run
//! manually in a terminal environment.

#![cfg(unix)]

use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
#[ignore = "shell requires a PTY (not available in CI)"]
fn should_run_single_command_via_shell(ctx: ManagerCtx) {
    // distant shell -- echo hello
    ctx.new_assert_cmd(["shell"])
        .args(["--", "echo", "hello"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hello"));
}

#[rstest]
#[test_log::test]
#[ignore = "shell requires a PTY (not available in CI)"]
fn should_forward_exit_code(ctx: ManagerCtx) {
    // distant shell -- bash -c 'exit 42'
    ctx.new_assert_cmd(["shell"])
        .args(["--", "bash", "-c", "exit 42"])
        .assert()
        .code(42);
}

#[rstest]
#[test_log::test]
#[ignore = "shell requires a PTY (not available in CI)"]
fn should_support_current_dir(ctx: ManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();

    let output = ctx
        .new_assert_cmd(["shell"])
        .arg("--current-dir")
        .arg(temp.path())
        .args(["--", "pwd"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let canonical = temp.path().canonicalize().unwrap();
    assert!(
        stdout.trim().contains(canonical.to_str().unwrap()),
        "Expected shell cwd to be {canonical:?}, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
#[ignore = "shell requires a PTY (not available in CI)"]
fn should_support_environment(ctx: ManagerCtx) {
    // distant shell --environment 'FOO="bar"' -- printenv FOO
    ctx.new_assert_cmd(["shell"])
        .args(["--environment", "FOO=\"bar\"", "--", "printenv", "FOO"])
        .assert()
        .success()
        .stdout(predicates::str::contains("bar"));
}
