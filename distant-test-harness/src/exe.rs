//! Helpers for building and locating test-harness binaries.

use std::io;
use std::path::PathBuf;

use crate::manager::build_dir;

/// Build the `tcp-to-stdio` binary and return its path.
///
/// Delegates to [`build_harness_bin`] with `"tcp-to-stdio"`.
pub async fn build_tcp_to_stdio() -> io::Result<PathBuf> {
    build_harness_bin("tcp-to-stdio").await
}

/// Build the `tcp-echo-server` binary and return its path.
///
/// Delegates to [`build_harness_bin`] with `"tcp-echo-server"`.
pub async fn build_tcp_echo_server() -> io::Result<PathBuf> {
    build_harness_bin("tcp-echo-server").await
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
