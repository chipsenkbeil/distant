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
    with containers; supports both Unix and Windows (`nanoserver`) containers
    via `DockerFamily` dispatch
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

## Memory Bank Maintenance (`CLAUDE.md` aka `AGENTS.md`)

> **Note:** `CLAUDE.md` is a symlink to `AGENTS.md`. Edits to either file
> automatically apply to both — no copying needed.

Prevent *context drift* by treating project documentation as a living journal:

1. **The Checkpoint Habit:** At the end of every session, run: *"Summarize
   architectural decisions made today and update AGENTS.md. Remove deprecated
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
   from resource contention — mitigated with nextest retries (4x), not
   root-caused.
3. **(Limitation)** `distant-docker` search on Windows containers uses
   `findstr.exe` which has limited regex support (no `+`, `?`, `|`, `{n,m}`).
   `Regex` and `Or` query conditions return `Unsupported` on Windows — only
   `Contains`, `Equals`, `StartsWith`, and `EndsWith` are supported.
4. **(Workaround)** `distant-docker` `append_file` on Windows falls back to tar
   read-modify-write because there's no good stdin-append equivalent to `cat >>`
   on nanoserver.
5. **(Limitation)** Nanoserver `ContainerUser` cannot write to
   `C:\Windows\Temp` or other system directories via exec commands (`mkdir`,
   `move`, etc.). The Docker tar API bypasses permissions but exec-based
   operations require a user-writable directory. Tests use `C:\temp`
   (pre-created via tar API in the test harness).
6. **(Workaround)** `distant-docker` tar directory creation uses a `.distant`
   zero-byte marker file workaround — Docker's `PUT /archive` API silently
   accepts directory-only tar archives on Windows nanoserver but never
   materializes the directories. The marker file forces creation; it is
   best-effort deleted afterward via exec, but may remain if the delete fails.
   A cleaner solution would require Docker engine changes.
7. **(Workaround)** `distant-docker` `rename` on Windows nanoserver falls back
   to tar-read + tar-write + exec-delete when the `move` command fails. This
   only works for files, not directories. The `move` failure appears to be a
   `ContainerUser` permission issue with directory entries created by Docker's
   overlay filesystem.
8. **(Workaround)** `distant-ssh` Windows `copy` uses a cmd.exe conditional
   (`if exist "src\*"`) to dispatch between `copy /Y` (files) and
   `xcopy /E /I /Y` (directories). `xcopy /I` treats the destination as a
   directory, which causes "Cannot perform a cyclic copy" when src and dst are
   sibling files in the same directory.
9. **(Bug)** Shell injection via unescaped paths — all `run_shell_cmd`
   calls in `distant-docker/src/api.rs` embed paths in single quotes
   without escaping `'`. A path containing a single-quote character
   (e.g. `/home/user/it's_a_file.txt`) causes a shell parse error and
   falls through to tar fallback silently. Fix: switch to direct argv via
   `run_cmd` where no shell features are needed, or add a
   `shell_quote()` helper.
10. **(Bug)** `distant-docker` `read_dir` on Windows always classifies
    all entries as `FileType::File` — `dir /b /a` provides no type
    information, and the code hardcodes `FileType::File` for every entry.
    Subdirectories are reported as files to the client. Fix: fall through
    to tar-based listing on Windows (which already handles types
    correctly).
11. **(Bug)** `distant-docker` `exists` on Windows — `if exist "path"`
    may not match directories depending on cmd.exe version, and the
    tar-based fallback is only reached on exec infrastructure failure (not
    on exit code 1). Fix: use both forms:
    `if exist "path\" (exit 0) else if exist "path" (exit 0) else (exit 1)`.
12. **(Bug)** `distant-docker` `remove` on Windows — the cmd.exe
    compound `rmdir ... 2>nul & if errorlevel 1 del /f ...` has subtle
    errorlevel propagation issues with the `&` operator. Fix: use
    `rmdir ... 2>nul || del /f ...` with `||` (run second only if first
    fails).
13. **(Bug)** Cross-platform path parsing in `distant-docker/src/utils.rs`
    — `tar_write_file`, `tar_create_dir`, and `tar_create_dir_all` use
    `std::path::Path` which parses with the host OS separator. Windows
    container paths like `C:\temp\file.txt` are misinterpreted when the
    Docker host is a Unix machine (backslash treated as literal, not
    separator). Fix: add a manual Windows-aware path splitter for
    container paths.
14. **(Bug)** `distant-docker` search shell injection — `path` is
    completely unquoted in search commands (`rg`, `grep`, `find`,
    `findstr`), and `shell_escape_pattern` doesn't escape single quotes.
    Fix: quote paths and escape `'` → `'\''` in patterns.
15. **(Limitation)** `distant-docker` `copy` tar fallback uses
    `tar_read_file` which skips directory entries — directory copies that
    fail via exec silently return `NotFound` from the fallback. Needs a
    `tar_copy_path` utility that handles both files and directories.
16. **(Limitation)** `auto_remove` on `LaunchOpts` is stored but never
    honored — launched containers are never cleaned up automatically.
    Needs `auto_remove: bool` on the `Docker` struct and lifecycle cleanup
    (e.g. on `ServerRef` drop or shutdown hook).
17. **(Limitation)** `distant-docker` search error handling — `grep`
    exit code 1 (no matches) vs exit code 2 (error) are not distinguished.
    A non-existent search path produces a silent empty result instead of
    an error. Fix: check exit code > 1 as an error, or use `set -o
    pipefail` in the shell command.
18. **(Acknowledgement)** OS detection inconsistency —
    `is_windows_image` (launch path) uses exact `== "windows"` while
    `detect_family_from_inspect` (connect path) uses
    `.contains("windows")`. Could misdetect if an image's OS field is
    something other than exactly `"windows"` (e.g. `"windows server"`).
    Fix: unify with a shared helper using `.contains("windows")`.

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

The nextest configuration lives in `.config/nextest.toml` and defines SSH test
throttling (`max-threads = 4` for `distant-ssh` tests), a retry policy (4
retries), and slow-timeout settings (60s period, terminate after 3 periods).

## Coding Style & Standards

To ensure the AI produces code that "feels right" we define the following
standards.

### General Patterns

1. **Workspace versioning:** Internal dependencies use exact version pinning (`version = "=0.20.0"`)
2. **Testing:** Use `rstest` for parameterized tests, `assert_fs` for filesystem tests
3. **Async runtime:** Tokio with full features
4. **Serialization:** Serde for JSON/TOML, MessagePack for protocol
5. **CLI:** Clap v4 with derive macros
6. **Logging:** Use `log` crate macros, configured via `flexi_logger`

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
8. Don't hardcode `sh -c` for shell commands in `distant-docker` — Windows
   containers (nanoserver) don't have `sh`. Use the `run_shell_cmd` /
   `run_shell_cmd_stdout` helpers which dispatch to `sh -c` or `cmd /c` based
   on `DockerFamily`.
9. Don't hardcode Unix paths like `/tmp` in Docker tests — use `test_temp_dir()`
   which returns `/tmp` on Unix and `C:\temp` on Windows. Don't use
   `C:\Windows\Temp` — `ContainerUser` cannot write there.
10. Don't double-wrap Windows commands with `cmd /c` — when using
    `run_shell_cmd` the shell prefix is already provided, so command strings
    should contain only the inner command (e.g. `mkdir "path"` not
    `cmd /c mkdir "path"`).
11. Don't use forward-slash separators inside `PathBuf::join()` for
    multi-component paths — `join("a/b/c")` embeds a Unix separator and
    can break on Windows. Use chained `.join("a").join("b").join("c")`.
12. When writing Docker tests (commands, paths, filenames), always consider
    Windows container compatibility — use `test_temp_dir()` for temp paths,
    chain `.join()` calls for path components, and use `cfg!(windows)` for
    any platform-specific commands or assertions.
13. Never bypass GPG commit signing — don't use `-c commit.gpgsign=false`
    or `--no-gpg-sign`. If `gpg failed to sign the data`, stop and let the
    user resolve the signing issue. The user's GPG key also handles SSH
    push authentication.
14. Never dismiss test failures as "intermittent" or "pre-existing" without
    investigation. Every failure — even if it only reproduces sometimes —
    must be analyzed to determine the root cause and a recommendation given
    to fix it. If the root cause is a bug in production code, focus the
    recommendation on the production code fix, not on test workarounds.
15. Don't rely on directory-only tar archives to create directories on Windows
    nanoserver via the Docker archive API — Docker silently accepts the upload
    but never materializes the directories. Always include a file entry (even a
    zero-byte marker) in the tar to force directory creation.
16. Don't use `xcopy /E /I /Y` for single-file copies on Windows via SSH — the
    `/I` flag makes xcopy treat the destination as a directory, causing "Cannot
    perform a cyclic copy" when src and dst are in the same directory. Use
    `copy /Y` for files and reserve `xcopy` for directory copies.
17. Don't assume exec-based `move` works reliably on Windows nanoserver — it
    can fail for files in directories created by Docker's overlay filesystem.
    Prefer a tar-read + tar-write + exec-delete fallback pattern for file
    renames when exec fails.

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

