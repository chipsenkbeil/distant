use assert_fs::prelude::*;
use indoc::indoc;
use predicates::Predicate;
use rstest::*;

use crate::cli::fixtures::*;

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
fn should_search_filesystem_using_query(ctx: DistantManagerCtx) {
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
        .stdout(stdout_predicate_fn)
        .stderr("");
}
