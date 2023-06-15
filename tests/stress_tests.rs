use assert_fs::prelude::*;
use predicates::prelude::*;
use rstest::*;

mod common;

use common::fixtures::*;

#[rstest]
#[test_log::test]
fn should_handle_large_volume_of_requests(ctx: DistantManagerCtx) {
    todo!();
}

#[rstest]
#[test_log::test]
fn should_handle_wide_spread_of_clients(ctx: DistantManagerCtx) {
    todo!();
}

#[rstest]
#[test_log::test]
fn should_handle_abrupt_client_disconnects(ctx: DistantManagerCtx) {
    todo!();
}

#[rstest]
#[test_log::test]
fn should_handle_badly_killing_client_shell_with_interactive_process(ctx: DistantManagerCtx) {
    todo!();
}
