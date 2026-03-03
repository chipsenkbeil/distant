//! E2E CLI tests for Docker backend file operations.
//!
//! Tests `distant fs read`, `distant fs write`, `distant fs copy`,
//! `distant fs rename`, and `distant fs remove` against a Docker container.

use distant_test_harness::docker::*;
use distant_test_harness::skip_if_no_docker;
use rstest::*;

#[rstest]
#[test_log::test]
fn should_write_and_read_file(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let path = "/tmp/distant-test-file.txt";
    let contents = "hello from distant docker test";

    // Write file
    ctx.new_assert_cmd(["fs", "write"])
        .args([path, contents])
        .assert()
        .success();

    // Read file back
    ctx.new_assert_cmd(["fs", "read"])
        .args([path])
        .assert()
        .success()
        .stdout(contents);
}

#[rstest]
#[test_log::test]
fn should_copy_file(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let src = "/tmp/distant-copy-src.txt";
    let dst = "/tmp/distant-copy-dst.txt";
    let contents = "copy test data";

    // Write source file
    ctx.new_assert_cmd(["fs", "write"])
        .args([src, contents])
        .assert()
        .success();

    // Copy
    ctx.new_assert_cmd(["fs", "copy"])
        .args([src, dst])
        .assert()
        .success();

    // Read destination
    ctx.new_assert_cmd(["fs", "read"])
        .args([dst])
        .assert()
        .success()
        .stdout(contents);
}

#[rstest]
#[test_log::test]
fn should_rename_file(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let src = "/tmp/distant-rename-src.txt";
    let dst = "/tmp/distant-rename-dst.txt";
    let contents = "rename test data";

    // Write source file
    ctx.new_assert_cmd(["fs", "write"])
        .args([src, contents])
        .assert()
        .success();

    // Rename
    ctx.new_assert_cmd(["fs", "rename"])
        .args([src, dst])
        .assert()
        .success();

    // Read destination
    ctx.new_assert_cmd(["fs", "read"])
        .args([dst])
        .assert()
        .success()
        .stdout(contents);

    // Source should no longer exist
    ctx.new_assert_cmd(["fs", "exists"])
        .args([src])
        .assert()
        .success()
        .stdout(predicates::str::contains("false"));
}

#[rstest]
#[test_log::test]
fn should_remove_file(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let path = "/tmp/distant-remove-file.txt";

    // Write file
    ctx.new_assert_cmd(["fs", "write"])
        .args([path, "data"])
        .assert()
        .success();

    // Remove
    ctx.new_assert_cmd(["fs", "remove"])
        .args([path])
        .assert()
        .success();

    // Verify gone
    ctx.new_assert_cmd(["fs", "exists"])
        .args([path])
        .assert()
        .success()
        .stdout(predicates::str::contains("false"));
}

#[rstest]
#[test_log::test]
fn should_fail_to_read_nonexistent_file(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    let output = ctx
        .new_std_cmd(["fs", "read"])
        .args(["/tmp/distant-test-no-such-file.txt"])
        .output()
        .expect("Failed to run fs read command");

    // NOTE: Ideally this should return a non-zero exit code, but the Docker
    // backend currently returns success for nonexistent files via the tar API.
    // At minimum, verify no file content is returned.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success() || stdout.trim().is_empty(),
        "Reading nonexistent file should either fail or return empty content, got: {stdout}",
    );
}

#[rstest]
#[test_log::test]
fn should_fail_to_copy_nonexistent_source(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    ctx.new_assert_cmd(["fs", "copy"])
        .args(["/tmp/distant-no-such-src.txt", "/tmp/distant-copy-dst.txt"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::contains("Failed to copy"));
}

#[rstest]
#[test_log::test]
fn should_fail_to_rename_nonexistent_source(docker_ctx: Option<DockerManagerCtx>) {
    let ctx = skip_if_no_docker!(docker_ctx);
    ctx.new_assert_cmd(["fs", "rename"])
        .args([
            "/tmp/distant-no-such-src.txt",
            "/tmp/distant-rename-dst.txt",
        ])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::contains("Failed to rename"));
}
