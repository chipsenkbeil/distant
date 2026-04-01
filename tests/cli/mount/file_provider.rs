//! Integration tests for the macOS FileProvider mount backend.
//!
//! These tests build a signed `.app` bundle with the test binary,
//! register it with PlugInKit, and verify it can function correctly.
//! Requires macOS and the `mount-macos-file-provider` feature.

use std::process::Command;

use super::build_test_app_bundle;

/// FP-01: The test app bundle should be structurally valid and contain
/// a working distant binary that responds to `--version`.
#[test]
fn file_provider_bundle_should_be_valid() {
    let bundle = build_test_app_bundle();
    let binary = bundle.join("Contents").join("MacOS").join("distant");

    assert!(binary.exists(), "bundled binary should exist");

    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .expect("should run bundled binary");

    assert!(
        output.status.success(),
        "bundled binary --version should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("distant"),
        "version output should contain 'distant', got: {stdout}"
    );
}

/// FP-02: The appex Info.plist should contain the test App Group ID
/// instead of the production one.
#[test]
fn file_provider_bundle_should_use_test_app_group() {
    let bundle = build_test_app_bundle();
    let appex_plist = bundle
        .join("Contents")
        .join("PlugIns")
        .join("DistantFileProvider.appex")
        .join("Contents")
        .join("Info.plist");

    assert!(appex_plist.exists(), "appex Info.plist should exist");

    let contents = std::fs::read_to_string(&appex_plist).expect("should read appex Info.plist");

    assert!(
        contents.contains("group.dev.distant.test"),
        "appex Info.plist should contain test App Group ID, got:\n{contents}"
    );
    assert!(
        !contents.contains("39C6AGD73Z.group.dev.distant"),
        "appex Info.plist should NOT contain production App Group ID"
    );
}

/// FP-03: Both the app and appex should be signed (codesign --verify should pass).
#[test]
fn file_provider_bundle_should_be_signed() {
    let bundle = build_test_app_bundle();

    let appex_path = bundle
        .join("Contents")
        .join("PlugIns")
        .join("DistantFileProvider.appex");

    // Verify appex signature
    let output = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&appex_path)
        .output()
        .expect("should run codesign --verify on appex");

    assert!(
        output.status.success(),
        "appex should have valid signature: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify app signature
    let output = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&bundle)
        .output()
        .expect("should run codesign --verify on app");

    assert!(
        output.status.success(),
        "app bundle should have valid signature: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
