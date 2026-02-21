use rstest::*;

use crate::common::fixtures::*;

#[rstest]
#[test_log::test]
fn should_output_version(ctx: DistantManagerCtx) {
    ctx.new_assert_cmd(vec!["manager", "version"])
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")));
}
