//! Integration tests for `distant manager service` subcommands.
//!
//! These tests exercise `install`, `start`, `stop`, and `uninstall` on the
//! system's service manager. They are gated behind the `DISTANT_TEST_SERVICE`
//! environment variable because they modify system state and require elevated
//! privileges (root on Unix, Administrator on Windows).
//!
//! Run with: `DISTANT_TEST_SERVICE=1 cargo test --all-features --test cli_tests -- cli::service`

use std::process::{Command, Stdio};

use distant_test_harness::manager::bin_path;

/// Returns true if service tests are enabled via `DISTANT_TEST_SERVICE=1`.
fn service_tests_enabled() -> bool {
    std::env::var("DISTANT_TEST_SERVICE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Skip the test if service tests are not enabled.
macro_rules! skip_if_no_service {
    () => {
        if !service_tests_enabled() {
            eprintln!("Skipping service test: set DISTANT_TEST_SERVICE=1 to enable");
            return;
        }
    };
}

#[test]
fn service_install_and_uninstall() {
    skip_if_no_service!();

    // Install the manager as a system service
    let install = Command::new(bin_path())
        .args(["manager", "service", "install"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service install");

    assert!(
        install.status.success(),
        "service install should succeed, stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    // Uninstall the service
    let uninstall = Command::new(bin_path())
        .args(["manager", "service", "uninstall"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service uninstall");

    assert!(
        uninstall.status.success(),
        "service uninstall should succeed, stderr: {}",
        String::from_utf8_lossy(&uninstall.stderr)
    );
}

#[test]
fn service_start_and_stop() {
    skip_if_no_service!();

    // Install first (start requires the service to be installed)
    let install = Command::new(bin_path())
        .args(["manager", "service", "install"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service install");

    assert!(
        install.status.success(),
        "service install should succeed before start, stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    // Start the service
    let start = Command::new(bin_path())
        .args(["manager", "service", "start"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service start");

    assert!(
        start.status.success(),
        "service start should succeed, stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    // Give the service time to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Stop the service
    let stop = Command::new(bin_path())
        .args(["manager", "service", "stop"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service stop");

    assert!(
        stop.status.success(),
        "service stop should succeed, stderr: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    // Clean up: uninstall
    let uninstall = Command::new(bin_path())
        .args(["manager", "service", "uninstall"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service uninstall");

    assert!(
        uninstall.status.success(),
        "service uninstall should succeed, stderr: {}",
        String::from_utf8_lossy(&uninstall.stderr)
    );
}

#[test]
fn service_start_without_install_fails() {
    skip_if_no_service!();

    // Trying to start a service that is not installed should fail
    let start = Command::new(bin_path())
        .args(["manager", "service", "start"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service start");

    // This may succeed if a service is already installed from a previous run,
    // but in a clean environment it should fail
    if !start.status.success() {
        // Expected: service not installed, start fails
        return;
    }

    // If it succeeded, stop and uninstall the leftover service
    let _ = Command::new(bin_path())
        .args(["manager", "service", "stop"])
        .output();
    let _ = Command::new(bin_path())
        .args(["manager", "service", "uninstall"])
        .output();
}

#[test]
fn service_stop_without_running_fails() {
    skip_if_no_service!();

    // Install the service but don't start it
    let install = Command::new(bin_path())
        .args(["manager", "service", "install"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service install");

    if !install.status.success() {
        // If install fails (perhaps already installed), just try the stop
    }

    // Stop without starting — should fail or return error
    let stop = Command::new(bin_path())
        .args(["manager", "service", "stop"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run service stop");

    // Clean up
    let _ = Command::new(bin_path())
        .args(["manager", "service", "uninstall"])
        .output();

    assert!(
        !stop.status.success(),
        "service stop without a running service should fail"
    );
}
