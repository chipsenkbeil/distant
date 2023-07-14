use rstest::*;

use crate::common::fixtures::*;

#[rstest]
#[test_log::test]
fn should_output_version(ctx: DistantManagerCtx) {
    // distant action capabilities
    ctx.new_assert_cmd(vec!["manager", "version"])
        .assert()
        .success()
        .stdout("WRONG")
        .stderr("");
}

#[rstest]
#[test_log::test]
fn should_support_output_version_as_json(ctx: DistantManagerCtx) {
    // distant action capabilities
    ctx.new_assert_cmd(vec!["manager", "version", "--format", "json"])
        .assert()
        .success()
        .stdout("WRONG")
        .stderr("");
}
