use assert_fs::prelude::*;
use distant_test_harness::manager::*;
use rstest::*;

#[rstest]
#[test_log::test]
#[ignore]
fn should_handle_large_volume_of_requests(ctx: ManagerCtx) {
    // Create a temporary directory to house a file we create and edit
    // with a large volume of requests
    let root = assert_fs::TempDir::new().unwrap();

    // Establish a path to a file we will edit repeatedly
    let path = root.child("file").to_path_buf();

    // Perform many requests of writing a file and reading a file
    for i in 1..100 {
        let _ = ctx
            .new_assert_cmd(["fs", "write"])
            .arg(path.to_str().unwrap())
            .write_stdin(format!("idx: {i}"))
            .assert();

        ctx.new_assert_cmd(["fs", "read"])
            .arg(path.to_str().unwrap())
            .assert()
            .stdout(format!("idx: {i}"));
    }
}

#[rstest]
#[test_log::test]
#[ignore]
fn should_handle_wide_spread_of_clients(_ctx: ManagerCtx) {
    todo!();
}

#[rstest]
#[test_log::test]
#[ignore]
fn should_handle_abrupt_client_disconnects(_ctx: ManagerCtx) {
    todo!();
}

#[rstest]
#[test_log::test]
#[ignore]
fn should_handle_badly_killing_client_shell_with_interactive_process(_ctx: ManagerCtx) {
    todo!();
}
