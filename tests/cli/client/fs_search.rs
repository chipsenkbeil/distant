//! Integration tests for the `distant fs search` CLI subcommand.
//!
//! Tests searching file contents using regex patterns.

use assert_fs::prelude::*;
use indoc::indoc;
use predicates::Predicate;
use rstest::*;

use distant_test_harness::manager::*;

const SEARCH_RESULTS_REGEX: &str = indoc! {r"
.*?[\\/]file1.txt
1:some file text

.*?[\\/]file2.txt
3:textual

.*?[\\/]file3.txt
1:more content
"};

#[rstest]
#[test_log::test]
fn should_search_filesystem_using_query(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("file1.txt").write_str("some file text").unwrap();
    root.child("file2.txt")
        .write_str("lines\nof\ntextual\ninformation")
        .unwrap();
    root.child("file3.txt").write_str("more content").unwrap();

    let stdout_predicate_fn = predicates::function::function(|s: &[u8]| {
        let s = std::str::from_utf8(s).unwrap();

        // Split by empty line, sort, and then rejoin with empty line inbetween
        let mut lines = s
            .split("\n\n")
            .map(|lines| lines.trim_end())
            .collect::<Vec<_>>();
        lines.sort_unstable();

        // Put together sorted text lines
        let full_text = format!("{}\n", lines.join("\n\n"));

        // Verify that it matches our search results regex
        let regex_fn = predicates::str::is_match(SEARCH_RESULTS_REGEX).unwrap();

        regex_fn.eval(&full_text)
    });

    // distant action search
    ctx.new_assert_cmd(["fs", "search"])
        .arg("te[a-z]*\\b")
        .arg(root.path())
        .assert()
        .success()
        .stdout(stdout_predicate_fn);
}

#[rstest]
#[test_log::test]
fn should_support_target_path(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("matching_name.txt")
        .write_str("irrelevant")
        .unwrap();
    root.child("other.log").write_str("irrelevant").unwrap();

    // distant fs search --target path 'matching' {path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--target", "path", "matching"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
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
#[test_log::test]
fn should_support_include_filter(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("file.txt").write_str("hello world").unwrap();
    root.child("file.log").write_str("hello world").unwrap();

    // distant fs search --include '\.txt$' 'hello' {path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--include", r"\.txt$", "hello"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("file.txt"),
        "Expected include to keep .txt file, got: {stdout}"
    );
    assert!(
        !stdout.contains("file.log"),
        "Expected include to filter out .log file, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_exclude_filter(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("keep.txt").write_str("hello world").unwrap();
    root.child("skip.txt").write_str("hello world").unwrap();

    // distant fs search --exclude 'skip' 'hello' {path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--exclude", "skip", "hello"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("keep.txt"),
        "Expected exclude to keep keep.txt, got: {stdout}"
    );
    assert!(
        !stdout.contains("skip.txt"),
        "Expected exclude to skip skip.txt, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_limit_option(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("a.txt").write_str("match here").unwrap();
    root.child("b.txt").write_str("match here").unwrap();
    root.child("c.txt").write_str("match here").unwrap();

    // distant fs search --limit 1 'match' {path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--limit", "1", "match"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // With limit 1, we should see at most one file path in the output
    let file_count = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .filter(|f| stdout.contains(*f))
        .count();
    assert_eq!(
        file_count, 1,
        "Expected exactly 1 file with --limit 1, got {file_count}: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_max_depth_option(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("top.txt").write_str("findme").unwrap();
    let sub = root.child("sub");
    sub.create_dir_all().unwrap();
    sub.child("deep.txt").write_str("findme").unwrap();

    // distant fs search --max-depth 1 'findme' {path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--max-depth", "1", "findme"])
        .arg(root.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("top.txt"),
        "Expected max-depth 1 to find top.txt, got: {stdout}"
    );
    assert!(
        !stdout.contains("deep.txt"),
        "Expected max-depth 1 to skip sub/deep.txt, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_support_upward_search(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("marker.txt").write_str("anchor").unwrap();
    let sub = root.child("sub");
    sub.create_dir_all().unwrap();

    // Search from sub directory upward — should find marker.txt in parent
    // distant fs search --upward --target path 'marker' {sub_path}
    let output = ctx
        .new_assert_cmd(["fs", "search"])
        .args(["--upward", "--target", "path", "marker"])
        .arg(sub.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("marker.txt"),
        "Expected upward search to find marker.txt in parent, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn should_return_no_results_for_nonmatching_pattern(ctx: ManagerCtx) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("file.txt").write_str("hello world").unwrap();

    // distant fs search 'zzz_nonexistent_pattern_zzz' {path}
    ctx.new_assert_cmd(["fs", "search"])
        .arg("zzz_nonexistent_pattern_zzz")
        .arg(root.path())
        .assert()
        .success()
        .stdout("");
}
