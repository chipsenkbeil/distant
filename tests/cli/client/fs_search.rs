//! Integration tests for the `distant fs search` CLI subcommand.
//!
//! Tests searching file contents using regex patterns.
//! Runs against Host, SSH, and Docker backends.

use rstest::*;

use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_find_matching_content(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(
        &ctx.child_path(&dir, "needle.txt"),
        "haystack needle haystack",
    );
    ctx.cli_write(&ctx.child_path(&dir, "other.txt"), "no match here");

    let output = ctx
        .new_std_cmd(["fs", "search"])
        .arg("needle")
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(
        output.status.success(),
        "fs search should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("needle.txt"),
        "Expected 'needle.txt' in search results, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_support_target_path(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search-path");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "matching_name.txt"), "irrelevant");
    ctx.cli_write(&ctx.child_path(&dir, "other.log"), "irrelevant");

    let output = ctx
        .new_std_cmd(["fs", "search"])
        .args(["--target", "path", "matching"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("matching_name.txt"),
        "Expected path search to find matching_name.txt, got: {stdout}"
    );
    assert!(
        !stdout.contains("other.log"),
        "Path search should not match other.log, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn should_return_no_results_for_nonmatching_pattern(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search-nomatch");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "file.txt"), "hello world");

    ctx.new_assert_cmd(["fs", "search"])
        .arg("zzz_nonexistent_pattern_zzz")
        .arg(&dir)
        .assert()
        .success()
        .stdout("");
}

/// Docker is excluded because its search implementation does not support
/// include/exclude filters.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_include_filter(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search-include");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "file.txt"), "hello world");
    ctx.cli_write(&ctx.child_path(&dir, "file.log"), "hello world");

    let output = ctx
        .new_std_cmd(["fs", "search"])
        .args(["--include", r"\.txt$", "hello"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("file.txt"),
        "Expected include to keep .txt file, got: {stdout}"
    );
    assert!(
        !stdout.contains("file.log"),
        "Expected include to filter out .log file, got: {stdout}"
    );
}

/// Docker is excluded because its search implementation does not support
/// include/exclude filters.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_exclude_filter(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search-exclude");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "keep.txt"), "hello world");
    ctx.cli_write(&ctx.child_path(&dir, "skip.txt"), "hello world");

    let output = ctx
        .new_std_cmd(["fs", "search"])
        .args(["--exclude", "skip", "hello"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("keep.txt"),
        "Expected exclude to keep keep.txt, got: {stdout}"
    );
    assert!(
        !stdout.contains("skip.txt"),
        "Expected exclude to skip skip.txt, got: {stdout}"
    );
}

/// Docker is excluded because its search implementation does not support
/// the max-depth option.
#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[test_log::test]
fn should_support_max_depth_option(#[case] backend: Backend) {
    if cfg!(windows) && matches!(backend, Backend::Ssh) {
        return; // SSH search requires Unix tools unavailable on Windows
    }
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("search-depth");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "top.txt"), "findme");
    let sub = ctx.child_path(&dir, "sub");
    ctx.cli_mkdir(&sub);
    ctx.cli_write(&ctx.child_path(&sub, "deep.txt"), "findme");

    let output = ctx
        .new_std_cmd(["fs", "search"])
        .args(["--max-depth", "1", "findme"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(
        output.status.success(),
        "fs search with max-depth should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("top.txt"),
        "Expected max-depth 1 to find top.txt, got: {stdout}"
    );
    assert!(
        !stdout.contains("deep.txt"),
        "Expected max-depth 1 to skip sub/deep.txt, got: {stdout}"
    );
}
