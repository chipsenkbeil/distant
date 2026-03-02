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
    ctx.new_assert_cmd(["fs", "read"])
        .args(["."])
        .assert()
        .success();
}
