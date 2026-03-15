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
