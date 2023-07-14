use distant_core::protocol::semver;
use distant_core::protocol::PROTOCOL_VERSION;
use rstest::*;

use crate::common::fixtures::*;
use crate::common::utils::TrimmedLinesMatchPredicate;

#[rstest]
#[test_log::test]
fn should_output_capabilities(ctx: DistantManagerCtx) {
    // Because all of our crates have the same version, we can expect it to match
    let version: semver::Version = env!("CARGO_PKG_VERSION").parse().unwrap();

    // Add the package name to the client version information
    let client_version = if version.build.is_empty() {
        let mut version = version.clone();
        version.build = semver::BuildMetadata::new(env!("CARGO_PKG_NAME")).unwrap();
        version
    } else {
        let mut version = version.clone();
        let raw_build_str = format!("{}.{}", version.build.as_str(), env!("CARGO_PKG_NAME"));
        version.build = semver::BuildMetadata::new(&raw_build_str).unwrap();
        version
    };

    // Add the distant-local to the server version information
    let server_version = if version.build.is_empty() {
        let mut version = version;
        version.build = semver::BuildMetadata::new("distant-local").unwrap();
        version
    } else {
        let raw_build_str = format!("{}.{}", version.build.as_str(), "distant-local");
        let mut version = version;
        version.build = semver::BuildMetadata::new(&raw_build_str).unwrap();
        version
    };

    // Since our client and server are built the same, all capabilities should be listed with +
    // and using 4 columns since we are not using a tty
    let expected = indoc::formatdoc! {"
        Client: {client_version} (Protocol {PROTOCOL_VERSION})
        Server: {server_version} (Protocol {PROTOCOL_VERSION})
        Capabilities supported (+) or not (-):
        +exec      +fs_io     +fs_perm   +fs_search
        +fs_watch  +sys_info
    "};

    ctx.cmd("version")
        .assert()
        .success()
        .stdout(TrimmedLinesMatchPredicate::new(expected))
        .stderr("");
}
