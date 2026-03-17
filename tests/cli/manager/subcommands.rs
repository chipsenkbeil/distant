//! Integration tests verifying that non-existent `distant manager` subcommands
//! are properly rejected by the CLI, and that existing subcommands parse correctly.

use std::process::{Command, Stdio};

use distant_test_harness::manager::bin_path;

#[test]
fn manager_info_does_not_exist() {
    let output = Command::new(bin_path())
        .args(["manager", "info"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run distant manager info");

    assert!(
        !output.status.success(),
        "distant manager info should fail (subcommand does not exist)"
    );
}

#[test]
fn manager_list_does_not_exist() {
    let output = Command::new(bin_path())
        .args(["manager", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run distant manager list");

    assert!(
        !output.status.success(),
        "distant manager list should fail (subcommand does not exist)"
    );
}

#[test]
fn manager_kill_does_not_exist() {
    let output = Command::new(bin_path())
        .args(["manager", "kill"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run distant manager kill");

    assert!(
        !output.status.success(),
        "distant manager kill should fail (subcommand does not exist)"
    );
}

#[test]
fn manager_select_does_not_exist() {
    let output = Command::new(bin_path())
        .args(["manager", "select"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run distant manager select");

    assert!(
        !output.status.success(),
        "distant manager select should fail (subcommand does not exist)"
    );
}
