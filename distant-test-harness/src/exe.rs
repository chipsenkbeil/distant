//! Helpers for building and locating test-harness binaries.

use std::io;
use std::path::PathBuf;

use crate::manager::build_dir;

/// Build the `tcp-to-stdio` binary and return its path.
pub async fn build_tcp_to_stdio() -> io::Result<PathBuf> {
    build_harness_bin("tcp-to-stdio", None).await
}

/// Build the `tcp-echo-server` binary and return its path.
pub async fn build_tcp_echo_server() -> io::Result<PathBuf> {
    build_harness_bin("tcp-echo-server", None).await
}

/// Build the `pty-echo` binary and return its path.
pub async fn build_pty_echo() -> io::Result<PathBuf> {
    build_harness_bin("pty-echo", None).await
}

/// Build the `pty-interactive` binary and return its path.
pub async fn build_pty_interactive() -> io::Result<PathBuf> {
    build_harness_bin("pty-interactive", None).await
}

/// Build the `pty-password` binary and return its path.
pub async fn build_pty_password() -> io::Result<PathBuf> {
    build_harness_bin("pty-password", None).await
}

/// Builds a named binary from the test harness crate and returns its path.
///
/// When `target` is `Some`, the binary is cross-compiled for the given triple
/// and placed under `target/<triple>/debug/` instead of the default build
/// directory. This is used for building binaries that need to run inside Docker
/// containers (e.g., Linux binaries built from a Linux host with a matching
/// cross-linker installed).
pub async fn build_harness_bin(bin_name: &str, target: Option<&str>) -> io::Result<PathBuf> {
    let mut cmd = tokio::process::Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "distant-test-harness", "--bin", bin_name]);

    if let Some(triple) = target {
        cmd.args(["--target", triple]);
    }

    let status = cmd.status().await?;

    if !status.success() {
        let detail = match target {
            Some(triple) => {
                format!("cargo build {bin_name} --target {triple} failed with {status}")
            }
            None => format!("cargo build {bin_name} failed with {status}"),
        };
        return Err(io::Error::other(detail));
    }

    match target {
        Some(triple) => {
            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let workspace_root = manifest_dir.parent().expect("workspace root");
            let target_dir = std::env::var("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| workspace_root.join("target"));

            // Cross-compiled binaries are always ELF (no .exe) since Docker containers are Linux
            Ok(target_dir.join(triple).join("debug").join(bin_name))
        }
        None => {
            let name = if cfg!(windows) {
                format!("{bin_name}.exe")
            } else {
                bin_name.to_string()
            };
            Ok(build_dir().join(name))
        }
    }
}
