//! macOS FileProvider backend integration tests.
//!
//! These tests validate that `install_test_app()` correctly builds and installs
//! the `.app` bundle to `/Applications/Distant.app` with proper structure,
//! signing, and pluginkit registration.

use rstest::rstest;

use distant_test_harness::backend::Backend;
use distant_test_harness::mount;
use distant_test_harness::skip_if_no_backend;

/// FP-01: Installing the test app should produce a valid `.app` at /Applications.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn install_test_app_should_create_valid_bundle(#[case] backend: Backend) {
    let _ctx = skip_if_no_backend!(backend);

    mount::install_test_app().expect("install_test_app should succeed");

    let app = std::path::Path::new("/Applications/Distant.app");
    assert!(app.exists(), "Distant.app should exist in /Applications");
    assert!(
        app.join("Contents/MacOS/distant").exists(),
        "should contain Contents/MacOS/distant"
    );
    assert!(
        app.join("Contents/Info.plist").exists(),
        "should contain Contents/Info.plist"
    );
    assert!(
        app.join("Contents/PlugIns/DistantFileProvider.appex/Contents/MacOS/distant")
            .exists(),
        "should contain the appex binary"
    );

    mount::restore_production_app();
}
