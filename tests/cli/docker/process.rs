//! E2E CLI tests for Docker backend process execution.
//!
//! Tests `distant spawn` against a Docker container.

use distant_test_harness::docker::*;
use distant_test_harness::skip_if_no_docker;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_execute_command(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "echo", "hello-from-docker"])
        .output()
        .expect("Failed to run spawn command");

    assert!(
        output.status.success(),
        "spawn exited with failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello-from-docker"),
        "expected 'hello-from-docker' in stdout, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_capture_stdout_from_process(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "ls", "/"])
        .output()
        .expect("Failed to run spawn command");

    assert!(
        output.status.success(),
        "ls / failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("etc"),
        "expected root directory listing to contain 'etc', got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_fail_for_nonexistent_binary(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "/usr/bin/distant-no-such-binary"])
        .output()
        .expect("Failed to run spawn command");

    assert!(
        !output.status.success(),
        "spawn of nonexistent binary should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("No such file"),
        "Expected error about missing binary, got stderr: {stderr}",
    );
}
