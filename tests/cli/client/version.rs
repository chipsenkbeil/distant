use distant_core::protocol::PROTOCOL_VERSION;
use rstest::*;

use crate::common::fixtures::*;
use crate::common::utils::TrimmedLinesMatchPredicate;

#[rstest]
#[test_log::test]
fn should_output_capabilities(ctx: DistantManagerCtx) {
    // Because all of our crates have the same version, we can expect it to match
    let package_name = "distant-local";
    let package_version = env!("CARGO_PKG_VERSION");
    let (major, minor, patch) = PROTOCOL_VERSION;

    // Since our client and server are built the same, all capabilities should be listed with +
    // and using 4 columns since we are not using a tty
    let expected = indoc::formatdoc! {"
        Client: distant {package_version} (Protocol {major}.{minor}.{patch})
        Server: {package_name} {package_version} (Protocol {major}.{minor}.{patch})
        Capabilities supported (+) or not (-):
        +cancel_search    +copy             +dir_create       +dir_read
        +exists           +file_append      +file_append_text +file_read
        +file_read_text   +file_write       +file_write_text  +metadata
        +proc_kill        +proc_resize_pty  +proc_spawn       +proc_stdin
        +remove           +rename           +search           +set_permissions
        +system_info      +unwatch          +version          +watch
    "};

    ctx.cmd("version")
        .assert()
        .success()
        .stdout(TrimmedLinesMatchPredicate::new(expected))
        .stderr("");
}
