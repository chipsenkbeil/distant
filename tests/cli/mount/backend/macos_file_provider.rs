//! macOS FileProvider backend integration tests.
//!
//! These tests validate that the `.app` bundle is correctly constructed and
//! that FileProvider domain registration works as expected.

use rstest::rstest;

use distant_test_harness::backend::Backend;
use distant_test_harness::mount;
use distant_test_harness::skip_if_no_backend;

/// FP-01: The test app bundle should be buildable and produce a valid `.app`
/// directory structure.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn app_bundle_should_build_successfully(#[case] backend: Backend) {
    let _ctx = skip_if_no_backend!(backend);

    let app_dir = mount::build_test_app_bundle();

    assert!(
        app_dir.exists(),
        "app bundle should exist at {}",
        app_dir.display()
    );
    assert!(
        app_dir
            .join("Contents")
            .join("MacOS")
            .join("distant")
            .exists(),
        "app bundle should contain Contents/MacOS/distant"
    );
}

/// FP-02: The test app bundle should contain an Info.plist in the main
/// Contents directory.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn app_bundle_should_have_info_plist(#[case] backend: Backend) {
    let _ctx = skip_if_no_backend!(backend);

    let app_dir = mount::build_test_app_bundle();
    let plist_path = app_dir.join("Contents").join("Info.plist");

    assert!(
        plist_path.exists(),
        "Info.plist should exist at {}",
        plist_path.display()
    );
}

/// FP-03: The test app bundle should contain the appex plugin directory
/// with the FileProvider extension binary.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn app_bundle_should_have_appex(#[case] backend: Backend) {
    let _ctx = skip_if_no_backend!(backend);

    let app_dir = mount::build_test_app_bundle();
    let appex_binary = app_dir
        .join("Contents")
        .join("PlugIns")
        .join("DistantFileProvider.appex")
        .join("Contents")
        .join("MacOS")
        .join("distant");

    assert!(
        appex_binary.exists(),
        "appex binary should exist at {}",
        appex_binary.display()
    );
}

/// FP-04: The appex Info.plist should use the test app group (without team
/// prefix) rather than the production app group.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn appex_plist_should_use_test_app_group(#[case] backend: Backend) {
    let _ctx = skip_if_no_backend!(backend);

    let app_dir = mount::build_test_app_bundle();
    let appex_plist = app_dir
        .join("Contents")
        .join("PlugIns")
        .join("DistantFileProvider.appex")
        .join("Contents")
        .join("Info.plist");

    let content = std::fs::read_to_string(&appex_plist)
        .unwrap_or_else(|e| panic!("failed to read appex Info.plist: {e}"));

    assert!(
        content.contains("group.dev.distant.test"),
        "appex plist should reference the test app group, got:\n{content}"
    );
    assert!(
        !content.contains("39C6AGD73Z.group.dev.distant"),
        "appex plist should NOT reference the production app group"
    );
}
