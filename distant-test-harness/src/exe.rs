//! Helpers for building and locating test-harness binaries.

use std::io;
use std::path::PathBuf;

use crate::manager::build_dir;

/// Build the `tcp-to-stdio` binary and return its path.
pub async fn build_tcp_to_stdio() -> io::Result<PathBuf> {
    build_harness_bin("tcp-to-stdio").await
}

/// Build the `tcp-echo-server` binary and return its path.
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
pub async fn build_harness_bin(bin_name: &str) -> io::Result<PathBuf> {
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

/// Builds a named binary from the test harness crate for a specific target triple.
///
/// The binary is placed under `target/<triple>/debug/` instead of the default
/// build directory. This is used for cross-compiling test binaries that need to
/// run inside Docker containers (e.g., Linux binaries built from macOS).
pub async fn build_harness_bin_for_target(bin_name: &str, target: &str) -> io::Result<PathBuf> {
    let status = tokio::process::Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "distant-test-harness",
            "--bin",
            bin_name,
            "--target",
            target,
        ])
        .status()
        .await?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo build {bin_name} --target {target} failed with {status}"
        )));
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));

    // Cross-compiled binaries are always ELF (no .exe) since Docker containers are Linux
    Ok(target_dir.join(target).join("debug").join(bin_name))
}
