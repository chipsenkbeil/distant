//! Integration tests for the `distant generate` CLI subcommand.
//!
//! Tests config generation (TOML output and file writing) and shell completion
//! generation for bash, zsh, and fish (stdout and file output).

use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn generate_config_should_output_valid_toml_to_stdout() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.args(["generate", "config"]).assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(!stdout.is_empty(), "Expected non-empty config output");

    // Parse the output as TOML to validate it is well-formed
    let parsed: toml::Value =
        toml::from_str(&stdout).expect("Generated config should be valid TOML");

    // Verify the parsed TOML is a table (top-level document)
    assert!(parsed.is_table(), "Expected TOML table at top level");

    // Check that the TOML contains expected config sections
    let table = parsed.as_table().unwrap();
    assert!(
        !table.is_empty(),
        "Expected non-empty TOML config, got empty table"
    );
}

#[test]
fn generate_config_should_write_to_file_when_output_specified() {
    let temp = assert_fs::TempDir::new().unwrap();
    let output_file = temp.child("config.toml");

    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["generate", "config", "--output"])
        .arg(output_file.path())
        .assert()
        .success();

    // Verify the file was created and has content
    output_file.assert(predicates::path::exists());
    let content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(!content.is_empty(), "Config file should not be empty");
}

#[test]
fn generate_completion_bash_should_output_to_stdout() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd
        .args(["generate", "completion", "bash"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(!stdout.is_empty(), "Expected non-empty completion output");
    // Bash completions typically contain function definitions
    assert!(
        stdout.contains("distant") || stdout.contains("complete") || stdout.contains("_distant"),
        "Expected bash completion content, got:\n{}",
        &stdout[..stdout.len().min(200)]
    );
}

#[test]
fn generate_completion_zsh_should_output_to_stdout() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd
        .args(["generate", "completion", "zsh"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        !stdout.is_empty(),
        "Expected non-empty zsh completion output"
    );
}

#[test]
fn generate_completion_should_write_to_file_when_output_specified() {
    let temp = assert_fs::TempDir::new().unwrap();
    let output_file = temp.child("completions.bash");

    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args(["generate", "completion", "bash", "--output"])
        .arg(output_file.path())
        .assert()
        .success();

    output_file.assert(predicates::path::exists());
    let content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(!content.is_empty(), "Completion file should not be empty");
}

#[test]
fn generate_completion_fish_should_output_to_stdout() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd
        .args(["generate", "completion", "fish"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        !stdout.is_empty(),
        "Expected non-empty fish completion output"
    );
}
