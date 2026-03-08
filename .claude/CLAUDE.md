# AI Development Workflow & Standards

This document outlines the workflow, tooling, and coding standards for
maintaining high-quality Rust project when collaborating with AI. Distant is a
Rust-based CLI tool for operating on remote computers through file and process
manipulation.

## Project Overview

- **Language:** Rust (Edition 2024, MSRV 1.88.0)
- **Architecture:** Cargo workspace with 5 member crates
- **Project Type:** CLI application with client-server architecture
- **Main Crates:**
  - `distant` - Main binary implementation providing commands like `distant
    api`, `distant connect`, and `distant launch`
  - `distant-core` - Core library with API, protocol, plugin trait, and
    utilities
  - `distant-docker` - Docker backend plugin using the Bollard API to interact
    with Unix containers (the Docker host can be any platform)
  - `distant-host` - Custom server implementation to run on a host machine that
    implements the full distant specification, also providing a client-side
    plugin
  - `distant-ssh` - pure Rust SSH client-side plugin compatible with distant
    specification
  - `distant-test-harness` - non-published crate that provides test utilities to
    run e2e system tests that involve standing up distant servers, managers,
    sshd, Docker containers, and more

## General AI Workflow

To move beyond basic code generation, use the following patterns:

1. **TDD-First Loop:** Before implementation, have the AI generate the test
   cases and the "Minimum Documentation Required" (MDR). Approve the contract
   before any production code is written.
2. **Recursive Refinement:** Instead of fixing "off" code manually, ask for a
   critique: *"Analyze this for Zero-Cost Abstractions. Provide three
   alternatives."*
3. **LSP-Context Injection:** Always provide current LSP diagnostics and
   compiler errors alongside code snippets to ground the AI in the project's
   current state.

### Common Pitfalls to Avoid

1. Don't commit without formatting via `cargo fmt --all`
2. Don't introduce clippy warnings (treat all warnings as errors)
3. Don't break cross-platform compatibility (test Unix and Windows code)
4. Don't modify workspace dependency versions without updating all members
5. Don't use outdated Rust patterns (prefer modern async/await over futures combinators)
6. Don't skip `--all-features` when testing (features are part of the API contract)

### Before Committing

1. **Format code:** `cargo fmt --all`
2. **Run linting:** `cargo clippy --all-features --workspace --all-targets`
3. **Run tests:** either all tests, or one of the individual crates, depending
   on what has changed.
    1. **All tests:** `cargo test --all-features --workspace`
    2. **Core tests:** `cargo test --all-features -p distant-core`
    3. **Docker tests:** `cargo test --all-features -p distant-docker`
    4. **Host tests:** `cargo test --all-features -p distant-host`
    5. **SSH tests:** `cargo test --all-features -p distant-ssh`

## Memory Bank Maintenance (`CLAUDE.md`)

> **Note:** The real file lives at `.claude/CLAUDE.md`. Root `CLAUDE.md` is a
> symlink for tool compatibility.

Prevent *context drift* by treating project documentation as a living journal:

1. **The Checkpoint Habit:** At the end of every session, run: *"Summarize
   architectural decisions made today and update CLAUDE.md. Remove deprecated
   patterns."*
2. **The Debt Ledger:** Maintain a `## Technical Debt` section. Every shortcut
   taken by the AI or yourself must be logged here to force acknowledgment in
   future tasks.
3. **Version Pinning:** Explicitly state the target versions for Rust (e.g.,
   1.88.0+) to avoid modern syntax being used in legacy-constrained
   environments.
4. **Anti-pattern Adjustments:** Maintain a `### Anti-Patterns` section. Every
   time a user provides the AI with feedback that corrects a decision, update
   the anti-patterns to reflect what was changed so the AI doesn't try that
   again.

## Technical Debt

Decisions we make that are considered shortcuts that we need to come back to
later to resolve will be placed here.

Each item is tagged with a category:

- **(Bug)** — Produces incorrect results, wrong behavior, or data corruption.
- **(Limitation)** — Missing or unsupported functionality that is known and
  intentionally deferred.
- **(Workaround)** — Correct behavior achieved through a non-ideal mechanism
  (e.g. fallback paths, platform shims). Works today but should be replaced
  with a cleaner solution.
- **(Acknowledgement)** — Known inconsistency or rough edge that is not
  currently causing failures but could in the future.

1. **(Limitation)** `win_service.rs` has `#![allow(dead_code)]` — Windows service integration
   may be incomplete/untested.
2. **(Acknowledgement)** Windows CI SSH tests have intermittent auth failures
   from resource contention — mitigated with nextest retries (4x). Root cause
   of the 19/47 consistent failures was the system sshd service running on the
   `windows-latest` VM, conflicting with per-test sshd instances; resolved by
   stopping the system service in CI.
3. **(Workaround)** `distant-ssh` Windows `copy` uses a cmd.exe conditional
   (`if exist "src\*"`) to dispatch between `copy /Y` (files) and
   `xcopy /E /I /Y` (directories). `xcopy /I` treats the destination as a
   directory, which causes "Cannot perform a cyclic copy" when src and dst are
   sibling files in the same directory.
4. **(Limitation)** Docker image pull has no CLI-visible progress — `info!`
   logs require `--log-level info` and go to the log file. Need a progress
   callback mechanism (e.g. `ManagerResponse::Progress`) for real-time spinner
   updates during long plugin operations like image pulls.
5. **(Bug)** Running `distant ssh` or `distant shell` after removing termwiz has
   resulted in programs like `nvim` (neovim) hanging and not displaying anything
   other than the cursor, or `ntop` (top on windows) hanging after the first
   visual display of the processes (no refresh, no time tick displayed).
6. **(Bug)** Config from ssh is not respected, at least when it comes to
   HostName. Performing `distant ssh windows-vm` fails to connect to
   ssh://windows-vm caused by, "SSH connection to windows-vm:22 failed: failed
   to lookup address information: nodename nor servname provided, or not known"

   An example of the config where regular `ssh windows-vm` would work:
   ```
   Include /Users/senkwich/.colima/ssh_config
   Host windows-vm
       HostName 10.211.55.3
       User senkwich
       IdentityFile ~/.ssh/id_windows_vm
       StrictHostKeyChecking no
       UserKnownHostsFile /dev/null
       ServerAliveInterval 60
       ServerAliveCountMax 3
    ```

## Tooling & Command Reference

### Building

```bash
# Standard build
cargo build

# Release build (highly optimized for size)
cargo build --release

# Build all workspace members with all features
cargo build --all-features --workspace

# Build specific crates
cargo build -p distant-core
cargo build -p distant-docker
cargo build -p distant-host
cargo build -p distant-ssh
```

### Formatting

```bash
# Format code (REQUIRED before committing)
cargo fmt --all
```

### Linting

```bash
# Run clippy on all targets (ensures test code is also linted)
cargo clippy --all-features --workspace --all-targets

# CI-style (treat warnings as errors, enable CI-specific settings)
# NOTE: Required before uploading to github for CI testing
RUSTFLAGS="-Dwarnings" cargo clippy --all-features --workspace --all-targets
```

### Testing

```bash
# Run all tests with all features across all crates/packages
cargo test --all-features --workspace

# Run doc tests
cargo test --all-features --workspace --doc

# Run a single test by name
cargo test --all-features -p <package> <test_name>

# Run a single test file (integration tests)
cargo test --all-features --test <test_file_name>

# Example: Run specific unit test
cargo test --all-features -p distant test_client_connect

# Example: Run specific integration test file
cargo test --all-features --test cli_tests
```

We also use `nextest` to run tests, especially on our CI:

```bash
# Install cargo nextest if unavailable
cargo install --locked cargo-nextest

# Run tests using nextest (preferred for CI-like behavior)
cargo nextest run --profile ci --all-features --workspace --all-targets
```

The nextest configuration lives in `.config/nextest.toml` and defines two test
groups: SSH throttling (`max-threads = 4` for `distant-ssh`) and Docker
throttling (`max-threads = 2` for `distant-docker`). These groups are assigned
via `[profile.default.overrides]`. The CI profile adds retries (4) and
slow-timeout (60s period, terminate after 3 periods).

### CI Workflows

Workflows live in `.github/workflows/`. Key workflows:

- **`ci.yml`** — Runs on every push/PR: format check, clippy, tests across
  ubuntu/macos/windows
- **`nightly.yml`** — Nightly release builds (scheduled at midnight UTC). Builds
  cross-compiled binaries for all release targets including Android
  (`aarch64-linux-android`). Supports `workflow_dispatch` for manual triggers.

```bash
# Manually trigger the nightly workflow (e.g. to test release builds)
gh workflow run "Nightly" --ref master

# Check the status of recent nightly runs
gh run list --workflow=nightly.yml --limit 5

# Watch a specific run
gh run watch <run-id>
```

## Coding Style & Standards

To ensure the AI produces code that "feels right" we define the following
standards.

### General Patterns

1. **Workspace versioning:** All crates use `version.workspace = true`; internal dependencies use exact version pinning via `[workspace.dependencies]` (e.g. `version = "=0.21.0-dev"`)
2. **Testing:** Use `rstest` for parameterized tests, `assert_fs` for filesystem tests
3. **Async runtime:** Tokio with full features
4. **Serialization:** Serde for JSON/TOML, MessagePack for protocol
5. **CLI:** Clap v4 with derive macros
6. **Logging:** Use `log` crate macros, configured via `flexi_logger`
7. **Remote paths:** Use `RemotePath` (not `PathBuf`) in all protocol and API
   signatures. `RemotePath` is a `String` newtype with no encoding assumptions —
   each plugin interprets paths in its own context (Docker = always Unix,
   SSH = auto-detected). Defined in `distant-core/src/protocol/common/remote_path.rs`,
   re-exported via `distant_core::protocol::RemotePath`.
8. **Plugin architecture:** Backend integrations implement the `Plugin` trait
   (`distant_core::Plugin`), which provides `connect()` and optionally
   `launch()`. Plugins receive raw destination strings (not parsed `Destination`
   structs), allowing non-standard URI formats like `docker://ubuntu:22.04`.
   Helpers `extract_scheme()` and `parse_destination()` are re-exported from
   `distant_core` for standard URI parsing. External process-based plugins use
   `ProcessPlugin` (see `PLUGINS.md`).

### Anti-Patterns

Keep a list of patterns to **avoid**:

1. Needless borrows in `#[cfg(windows)]` code — use `["arg1", "arg2"]` not
   `&["arg1", "arg2"]` with `.args()`, and `format!(...)` not `&format!(...)`
   with `.arg()`. These cause clippy warnings only visible in Windows CI.
2. Always verify Windows CI clippy output — `#[cfg(windows)]` blocks are
   invisible to local macOS clippy runs.
3. Don't spawn ssh-agent per-test — use direct key file loading to avoid fork
   exhaustion.
4. Don't run mass parallel SSH integration tests without throttling — use
   nextest `test-groups` with `max-threads` (configured in
   `.config/nextest.toml`).
5. Always create a **feature branch** before starting multi-file or multi-phase
   work — never commit directly to `master`. Use
   `git checkout -b feature/<name>` before writing any code.
6. **Commit per-phase** (or at minimum per logical unit of work) — don't
   accumulate an entire feature as uncommitted changes across many phases. Each
   phase should be a separate commit with `cargo fmt` and `cargo clippy` passing
   before the commit is created.
7. Always **run tests** (`cargo test --all-features -p <crate>`) after creating
   or modifying test files — don't assume tests compile or pass without actually
   executing them.
8. Don't use forward-slash separators inside `PathBuf::join()` for
   multi-component paths — `join("a/b/c")` embeds a Unix separator and
   can break on Windows. Use chained `.join("a").join("b").join("c")`.
9. Never bypass GPG commit signing — don't use `-c commit.gpgsign=false`
   or `--no-gpg-sign`. If `gpg failed to sign the data`, stop and let the
   user resolve the signing issue. The user's GPG key also handles SSH
   push authentication.
10. Never dismiss test failures as "intermittent" or "pre-existing" without
    investigation. Every failure — even if it only reproduces sometimes —
    must be analyzed to determine the root cause and a recommendation given
    to fix it. If the root cause is a bug in production code, focus the
    recommendation on the production code fix, not on test workarounds.
11. Don't use `xcopy /E /I /Y` for single-file copies on Windows via SSH — the
    `/I` flag makes xcopy treat the destination as a directory, causing "Cannot
    perform a cyclic copy" when src and dst are in the same directory. Use
    `copy /Y` for files and reserve `xcopy` for directory copies.

### Format standards

Always follow the standards set in the root-level `rustfmt.toml` file, and make
sure to run `cargo fmt --all`.

### Exports from crates

For library crates, avoid nested public exports like
`distant_core::client::UntypedClient` in favor of root-level imports like
`distant_core::UntypedClient`.

### Error handling

Use `anyhow` for flexible error handling in the binary crate code (not library
crates).

Leverage situation-specific error types for library crates like `distant-core`,
`distant-host`, or `distant-ssh`.

Make sure to provide context when propagating errors:

```rust
let config = load_config().context("Failed to load configuration file")?;
```

Custom error types use `derive_more` to simplify construction and reduce boilerplate

```rust
use derive_more::{Display, Error, From};

#[derive(Debug, Display, Error, From)]
pub enum CliError {
    #[display(fmt = "exit code: {}", _0)]
    Exit(#[error(not(source))] u8),

    #[display(fmt = "error: {}", _0)]
    Error(#[error(not(source))] anyhow::Error),
}
```

### Derive Macros

Use derive macros extensively:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Fields
}
```

### Type Aliases

Leverage type aliases to simplify complex results, associating the name into the result type

```rust
pub type CliResult = Result<(), CliError>;
```

### Naming

Use standard Rust naming conventions with the added requirements of:

1. Keep naming simple and rely on namespaces for full clarity (e.g. name
   `Client` instead of `DistantClient` for a struct in `distant_core::Client`)
2. Provide clear, meaningful names for variables, structs, traits, and more that
   follow well-known, established Rust crates
3. Prefix names with `_` if they are unused (including phantom markers that must
   exist but are not accessed)

### Documentation

Code should be well documented without introducing poor or unneeded
explanations. All public functions structs, and other items from library crates
MUST have documentation associated, which will include (where applicable)
examples, error explanations, and details about the parameters. Always start
with a one line explanation of structs, traits, fnctions, etc.

Here are examples of what looks good, not necessarily from the `distant`
project.

````rust
/// A slice of a path (akin to [`str`]).
///
/// This type supports a number of operations for inspecting a path, including
/// breaking the path into its components (separated by `/` on Unix and by either
/// `/` or `\` on Windows), extracting the file name, determining whether the path
/// is absolute, and so on.
///
/// This is an *unsized* type, meaning that it must always be used behind a
/// pointer like `&` or [`Box`]. For an owned version of this type,
/// see [`Utf8PathBuf`].
///
/// # Examples
///
/// ```
/// use typed_path::{Utf8Path, Utf8UnixEncoding};
///
/// // NOTE: A path cannot be created on its own without a defined encoding,
/// //       but all encodings work on all operating systems, providing the
/// //       ability to parse and operate on paths independently of the
/// //       compiled platform
/// let path = Utf8Path::<Utf8UnixEncoding>::new("./foo/bar.txt");
///
/// let parent = path.parent();
/// assert_eq!(parent, Some(Utf8Path::new("./foo")));
///
/// let file_stem = path.file_stem();
/// assert_eq!(file_stem, Some("bar"));
///
/// let extension = path.extension();
/// assert_eq!(extension, Some("txt"));
/// ```
///
/// In addition to explicitly using [`Utf8Encoding`]s, you can also
/// leverage aliases available from the crate to work with paths:
///
/// ```
/// use typed_path::{Utf8UnixPath, Utf8WindowsPath};
///
/// // Same as Utf8Path<Utf8UnixEncoding>
/// let path = Utf8UnixPath::new("/foo/bar.txt");
///
/// // Same as Utf8Path<Utf8WindowsEncoding>
/// let path = Utf8WindowsPath::new(r"C:\foo\bar.txt");
/// ```
///
/// To mirror the design of Rust's standard library, you can access
/// the path associated with the compiled rust platform using [`Utf8NativePath`],
/// which itself is an alias to one of the other choices:
///
/// ```
/// use typed_path::Utf8NativePath;
///
/// // On Unix, this would be Utf8UnixPath aka Utf8Path<Utf8UnixEncoding>
/// // On Windows, this would be Utf8WindowsPath aka Utf8Path<Utf8WindowsEncoding>
/// let path = Utf8NativePath::new("/foo/bar.txt");
/// ```
///
/// [`Utf8NativePath`]: crate::Utf8NativePath
#[repr(transparent)]
pub struct Utf8Path<T>
where
    T: Utf8Encoding,
{
    /// Encoding associated with path buf
    _encoding: PhantomData<T>,

    /// Path as an unparsed str slice
    pub(crate) inner: str,
}
````

````rust
/// Returns a path that, when joined onto `base`, yields `self`.
///
/// # Errors
///
/// If `base` is not a prefix of `self` (i.e., [`starts_with`]
/// returns `false`), returns [`Err`].
///
/// [`starts_with`]: Utf8Path::starts_with
///
/// # Examples
///
/// ```
/// use typed_path::{Utf8Path, Utf8PathBuf, Utf8UnixEncoding};
///
/// // NOTE: A path cannot be created on its own without a defined encoding
/// let path = Utf8Path::<Utf8UnixEncoding>::new("/test/haha/foo.txt");
///
/// assert_eq!(path.strip_prefix("/"), Ok(Utf8Path::new("test/haha/foo.txt")));
/// assert_eq!(path.strip_prefix("/test"), Ok(Utf8Path::new("haha/foo.txt")));
/// assert_eq!(path.strip_prefix("/test/"), Ok(Utf8Path::new("haha/foo.txt")));
/// assert_eq!(path.strip_prefix("/test/haha/foo.txt"), Ok(Utf8Path::new("")));
/// assert_eq!(path.strip_prefix("/test/haha/foo.txt/"), Ok(Utf8Path::new("")));
///
/// assert!(path.strip_prefix("test").is_err());
/// assert!(path.strip_prefix("/haha").is_err());
///
/// let prefix = Utf8PathBuf::<Utf8UnixEncoding>::from("/test/");
/// assert_eq!(path.strip_prefix(prefix), Ok(Utf8Path::new("haha/foo.txt")));
/// ```
pub fn strip_prefix<P>(&self, base: P) -> Result<&Utf8Path<T>, StripPrefixError>
where
    P: AsRef<Utf8Path<T>>,
{
    self._strip_prefix(base.as_ref())
}
````

### Platform-Specific Code

Distant runs on Windows, MacOS, Linux, and should also work on FreeBSD and other
BSD variants. `cfg` attributes will be used to isolate modules and imports that
only build and run on specific platforms, both in production code and test code.

```rust
// Use cfg attributes for platform-specific code
#[cfg(windows)]
pub mod win_service;

#[cfg(unix)]
use fork::daemon;

#[cfg_attr(unix, allow(unused_imports))]
pub(crate) use common::Spawner;

// Platform-specific dependencies in Cargo.toml
[target.'cfg(unix)'.dependencies]
fork = "0.1.21"

[target.'cfg(windows)'.dependencies]
windows-service = "0.6.0"
```

