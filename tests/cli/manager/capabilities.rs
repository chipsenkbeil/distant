use crate::cli::fixtures::*;
use indoc::indoc;
use rstest::*;

const EXPECTED_TABLE: &str = indoc! {"
+---------------+--------------------------------------------------------------+
| kind          | description                                                  |
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
| launch        | Supports launching distant on remote servers                 |
+---------------+--------------------------------------------------------------+
| list          | Supports retrieving a list of managed connections            |
+---------------+--------------------------------------------------------------+
| open_channel  | Supports opening a channel with a remote server              |
+---------------+--------------------------------------------------------------+
| shutdown      | Supports being shut down on demand                           |
+---------------+--------------------------------------------------------------+
"};

#[rstest]
fn should_output_capabilities(ctx: DistantManagerCtx) {
    // distant action capabilities
    ctx.new_assert_cmd(vec!["manager", "capabilities"])
        .assert()
        .success()
        .stdout(format!("{EXPECTED_TABLE}\n"))
        .stderr("");
}
