use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn should_output_version(ctx: ManagerCtx) {
    ctx.new_assert_cmd(vec!["manager", "version"])
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")));
}
