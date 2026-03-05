//! Build script that embeds git metadata into the binary.
//!
//! Sets the following environment variables (consumed via `option_env!()` at compile time):
//!
//! - `DISTANT_BUILD_GIT_HASH` — Short (10-char) commit hash of HEAD
//! - `DISTANT_BUILD_GIT_DIRTY` — `"true"` if the working tree has uncommitted changes
//! - `DISTANT_BUILD_DATE` — Date of the HEAD commit in `YYYY-MM-DD` format
//!
//! When git is unavailable (e.g. building from a tarball), all three are silently omitted
//! and `option_env!()` returns `None`, causing the CLI to fall back to the plain semver.

use std::process::Command;

fn main() {
    // Re-run this script when the git HEAD or refs change (i.e. new commits, branch switches).
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    // Attempt to read git metadata. If any command fails (no git, no .git dir, etc.),
    // we silently skip — the binary will show only the semver version.
    if let Some(hash) = git_short_hash() {
        println!("cargo:rustc-env=DISTANT_BUILD_GIT_HASH={hash}");

        // Only emit dirty/date if we successfully got the hash.
        if let Some(dirty) = git_is_dirty() {
            println!("cargo:rustc-env=DISTANT_BUILD_GIT_DIRTY={dirty}");
        }
        if let Some(date) = git_commit_date() {
            println!("cargo:rustc-env=DISTANT_BUILD_DATE={date}");
        }
    }
}

/// Returns the short (10-char) hash of HEAD, or `None` if git is unavailable.
fn git_short_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hash = String::from_utf8(output.stdout).ok()?;
    let hash = hash.trim();
    if hash.is_empty() {
        return None;
    }

    Some(hash.to_owned())
}

/// Returns `"true"` if the working tree is dirty, `"false"` otherwise.
fn git_is_dirty() -> Option<String> {
    let status = Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()
        .ok()?;

    // Exit code 0 = clean, non-zero = dirty.
    Some(if status.success() { "false" } else { "true" }.to_owned())
}

/// Returns the commit date of HEAD in `YYYY-MM-DD` format.
fn git_commit_date() -> Option<String> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%cs", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let date = String::from_utf8(output.stdout).ok()?;
    let date = date.trim();
    if date.is_empty() {
        return None;
    }

    Some(date.to_owned())
}
