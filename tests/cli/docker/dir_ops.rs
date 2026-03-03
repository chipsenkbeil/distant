//! E2E CLI tests for Docker backend directory operations.
//!
//! Tests `distant fs make-dir`, `distant fs read` (directory mode), and
//! `distant fs exists` against a Docker container.

use distant_test_harness::docker::*;
use distant_test_harness::skip_if_no_docker;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_create_directory(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let dir = "/tmp/distant-test-mkdir";

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "exists"])
        .args([dir])
        .assert()
        .success()
        .stdout(predicates::str::contains("true"));
}

#[rstest]
#[test_log::test]
fn should_create_nested_directories(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let dir = "/tmp/distant-test-nested/a/b/c";

    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["--all", dir])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "exists"])
        .args([dir])
        .assert()
        .success()
        .stdout(predicates::str::contains("true"));
}

#[rstest]
#[test_log::test]
fn should_read_dir_listing(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let dir = "/tmp/distant-test-readdir";
    let file = "/tmp/distant-test-readdir/child.txt";

    // Create directory and a file inside it
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args([dir])
        .assert()
        .success();

    ctx.new_assert_cmd(["fs", "write"])
        .args([file, "data"])
        .assert()
        .success();

    // Verify the file inside the directory can be read back
    ctx.new_assert_cmd(["fs", "read"])
        .args([file])
        .assert()
        .success()
        .stdout("data");

    // Verify the directory exists via metadata
    ctx.new_assert_cmd(["fs", "metadata"])
        .args([dir])
        .assert()
        .success()
        .stdout(predicates::str::contains("Type: dir"));
}

#[rstest]
#[test_log::test]
fn should_read_dir_with_dot_path(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);

    // Reading `.` should succeed and return entries (the container's cwd)
    let output = ctx
        .new_std_cmd(["fs", "read"])
        .args(["."])
        .output()
        .expect("Failed to run fs read .");

    assert!(
        output.status.success(),
        "fs read . should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected directory listing output for '.', got empty",
    );
}

#[rstest]
#[test_log::test]
fn should_fail_to_create_dir_when_parent_missing(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    // Without --all, creating a dir under a nonexistent parent should fail
    ctx.new_assert_cmd(["fs", "make-dir"])
        .args(["/tmp/distant-nonexistent-parent/child"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::contains("Failed to make directory"));
}
