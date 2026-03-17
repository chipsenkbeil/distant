//! Helpers for building and locating test-harness binaries.

use std::io;
use std::path::PathBuf;

use crate::manager::build_dir;

/// Build the `tcp-to-stdio` binary and return its path.
///
/// Runs `cargo build -p distant-test-harness --bin tcp-to-stdio` and then
/// returns the path inside the workspace target directory.
pub async fn build_tcp_to_stdio() -> io::Result<PathBuf> {
    let status = tokio::process::Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "distant-test-harness",
            "--bin",
            "tcp-to-stdio",
        ])
        .status()
        .await?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo build tcp-to-stdio failed with {status}"
        )));
    }

    let name = if cfg!(windows) {
        "tcp-to-stdio.exe"
    } else {
        "tcp-to-stdio"
    };

    Ok(build_dir().join(name))
}

/// Build the `tcp-echo-server` binary and return its path.
///
/// Runs `cargo build -p distant-test-harness --bin tcp-echo-server` and then
/// returns the path inside the workspace target directory.
pub async fn build_tcp_echo_server() -> io::Result<PathBuf> {
    let status = tokio::process::Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "distant-test-harness",
            "--bin",
            "tcp-echo-server",
        ])
        .status()
        .await?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo build tcp-echo-server failed with {status}"
        )));
    }

    let name = if cfg!(windows) {
        "tcp-echo-server.exe"
    } else {
        "tcp-echo-server"
    };

    Ok(build_dir().join(name))
}

/// Build the `pty-echo` binary and return its path.
pub async fn build_pty_echo() -> io::Result<PathBuf> {
    build_harness_bin("pty-echo").await
}

/// Build the `pty-interactive` binary and return its path.
pub async fn build_pty_interactive() -> io::Result<PathBuf> {
    build_harness_bin("pty-interactive").await
}

/// Build the `pty-password` binary and return its path.
pub async fn build_pty_password() -> io::Result<PathBuf> {
    build_harness_bin("pty-password").await
}

/// Builds a named binary from the test harness crate and returns its path.
async fn build_harness_bin(bin_name: &str) -> io::Result<PathBuf> {
    let status = tokio::process::Command::new(env!("CARGO"))
        .args(["build", "-p", "distant-test-harness", "--bin", bin_name])
        .status()
        .await?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo build {bin_name} failed with {status}"
        )));
    }

    let name = if cfg!(windows) {
        format!("{bin_name}.exe")
    } else {
        bin_name.to_string()
    };

    Ok(build_dir().join(name))
}
