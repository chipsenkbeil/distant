use crate::cli::fixtures::*;
use assert_cmd::Command;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::Predicate;
use rstest::*;
use serde_json::json;

const SEARCH_RESULTS_REGEX: &str = indoc! {r"
.*?[\\/]file1.txt
1:some file text

.*?[\\/]file2.txt
3:textual

.*?[\\/]file3.txt
1:more content
"};

#[rstest]
fn should_search_filesystem_using_query(mut action_cmd: CtxCommand<Command>) {
    let root = assert_fs::TempDir::new().unwrap();
    root.child("file1.txt").write_str("some file text").unwrap();
    root.child("file2.txt")
        .write_str("lines\nof\ntextual\ninformation")
        .unwrap();
    root.child("file3.txt").write_str("more content").unwrap();

    let query = json!({
        "path": root.path().to_string_lossy(),
        "target": "contents",
        "condition": {"type": "regex", "value": "te[a-z]*\\b"},
    });

    let stdout_predicate_fn = predicates::function::function(|s: &[u8]| {
        let s = std::str::from_utf8(s).unwrap();

        // Split by empty line, sort, and then rejoin with empty line inbetween
        let mut lines = s
            .split("\n\n")
            .map(|lines| lines.trim_end())
            .collect::<Vec<_>>();
        lines.sort_unstable();

        // Put together sorted text lines
        let full_text = lines.join("\n\n");

        // Verify that it matches our search results regex
        let regex_fn = predicates::str::is_match(SEARCH_RESULTS_REGEX).unwrap();

        regex_fn.eval(&full_text)
    });

    // distant action system-info
    action_cmd
        .arg("search")
        .arg(&serde_json::to_string(&query).unwrap())
        .assert()
        .success()
        .stdout(stdout_predicate_fn)
        .stderr("");
}
