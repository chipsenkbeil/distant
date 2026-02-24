//! Integration tests for CLI config handling.
//!
//! Tests `--config` flag parsing, invalid config handling, and config generation roundtrip.

use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn config_flag_with_invalid_toml_produces_error() {
    let temp = assert_fs::TempDir::new().unwrap();
    let bad_config = temp.child("bad.toml");
    bad_config.write_str("this is [[ not valid toml").unwrap();

    // Use a real subcommand (not --help) so the config is actually loaded
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.arg("--config")
        .arg(bad_config.path())
        .arg("generate")
        .arg("config")
        .assert()
        .failure();
}

#[test]
fn config_flag_with_nonexistent_file_produces_error() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    cmd.args([
        "--config",
        "/nonexistent/path/config.toml",
        "generate",
        "config",
    ])
    .assert()
    .failure();
}

#[test]
fn generate_config_roundtrip() {
    // Generate a config
    let mut gen_cmd: Command = assert_cmd::cargo_bin_cmd!();
    let gen_output = gen_cmd.args(["generate", "config"]).assert().success();

    let config_text = String::from_utf8_lossy(&gen_output.get_output().stdout);
    assert!(
        !config_text.is_empty(),
        "Generated config should not be empty"
    );

    // Write the generated config to a temp file
    let temp = assert_fs::TempDir::new().unwrap();
    let config_file = temp.child("generated.toml");
    config_file.write_str(&config_text).unwrap();

    // Verify it parses as valid TOML
    let parsed: toml::Value =
        toml::from_str(&config_text).expect("Generated config should be valid TOML");
    assert!(parsed.is_table(), "Config should be a TOML table");
}

#[test]
fn generate_config_contains_expected_sections() {
    let mut cmd: Command = assert_cmd::cargo_bin_cmd!();
    let output = cmd.args(["generate", "config"]).assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: toml::Value = toml::from_str(&stdout).expect("Should be valid TOML");
    let table = parsed.as_table().unwrap();

    // Verify expected top-level sections exist (actual sections from generate config)
    let expected_sections = ["client", "generate", "manager", "server"];

    for section in expected_sections {
        assert!(
            table.contains_key(section),
            "Expected config section '{section}' to be present. Sections found: {:?}",
            table.keys().collect::<Vec<_>>()
        );
    }
}
