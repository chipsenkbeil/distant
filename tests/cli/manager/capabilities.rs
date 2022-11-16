use crate::cli::fixtures::*;
use indoc::indoc;
use rstest::*;

const EXPECTED_TABLE: &str = indoc! {"
+---------------+--------------------------------------------------------------+
| kind          | description                                                  |
+---------------+--------------------------------------------------------------+
| authenticate  | Supports authenticating with a remote server                 |
+---------------+--------------------------------------------------------------+
| capabilities  | Supports retrieving capabilities                             |
+---------------+--------------------------------------------------------------+
| channel       | Supports sending data through a channel with a remote server |
+---------------+--------------------------------------------------------------+
| close_channel | Supports closing a channel with a remote server              |
+---------------+--------------------------------------------------------------+
| connect       | Supports connecting to remote servers                        |
+---------------+--------------------------------------------------------------+
| info          | Supports retrieving connection-specific information          |
+---------------+--------------------------------------------------------------+
| kill          | Supports killing a remote connection                         |
+---------------+--------------------------------------------------------------+
| launch        | Supports launching a server on remote machines               |
+---------------+--------------------------------------------------------------+
| list          | Supports retrieving a list of managed connections            |
+---------------+--------------------------------------------------------------+
| open_channel  | Supports opening a channel with a remote server              |
+---------------+--------------------------------------------------------------+
"};

#[rstest]
#[test_log::test]
fn should_output_capabilities(ctx: &'static DistantManagerCtx) {
    // distant action capabilities
    ctx.new_assert_cmd(vec!["manager", "capabilities"])
        .assert()
        .success()
        .stdout(format!("{EXPECTED_TABLE}\n"))
        .stderr("");
}
