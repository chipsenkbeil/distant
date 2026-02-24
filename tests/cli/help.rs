//! Integration tests for the `distant --help` and `distant manager --help` output.
//!
//! Verifies that top-level and manager subcommand help text contains expected
//! commands and does not expose removed/internal commands.

use assert_cmd::Command;

#[test]
fn distant_help_should_include_top_level_commands() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.arg("--help").assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    for cmd in ["status", "kill", "select", "ssh", "connect", "shell"] {
        assert!(
            stdout.contains(cmd),
            "Expected top-level help to contain '{cmd}', got:\n{stdout}"
        );
    }
}

#[test]
fn distant_help_should_not_include_removed_manager_commands_at_top_level() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.arg("--help").assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // "list" should not appear as a top-level command
    // Check that no line in the help starts with a command named "list"
    for line in stdout.lines() {
        let trimmed = line.trim();
        assert!(
            !trimmed.starts_with("list "),
            "Found 'list' as a top-level command in help:\n{stdout}"
        );
    }
}

#[test]
fn distant_manager_help_should_only_show_daemon_commands() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.args(["manager", "--help"]).assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    for cmd in ["listen", "version", "service"] {
        assert!(
            stdout.contains(cmd),
            "Expected manager help to contain '{cmd}', got:\n{stdout}"
        );
    }
    for cmd in ["list", "kill", "info", "select"] {
        for line in stdout.lines() {
            let trimmed = line.trim();
            assert!(
                !trimmed.starts_with(&format!("{cmd} ")),
                "Found '{cmd}' as a subcommand in manager help:\n{stdout}"
            );
        }
    }
}

#[test]
fn distant_manager_list_should_be_unknown_command() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["manager", "list"]).assert().failure();
}
