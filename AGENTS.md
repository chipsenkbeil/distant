# Agent Guidelines for Distant

This document provides coding guidelines and commands for AI coding agents working on the Distant project. Distant is a Rust-based CLI tool for operating on remote computers through file and process manipulation.

## Project Overview

- **Language:** Rust (Edition 2021, MSRV 1.70.0)
- **Architecture:** Cargo workspace with 6 member crates
- **Project Type:** CLI application with client-server architecture
- **Main Crates:**
  - `distant` - Main binary/CLI
  - `distant-auth` - Authentication handlers
  - `distant-core` - Core library with API/protocol
  - `distant-local` - Local API implementation
  - `distant-net` - Network layer
  - `distant-protocol` - Protocol data structures
  - `distant-ssh2` - SSH integration (optional)

## Build, Lint, and Test Commands

### Building
```bash
# Standard build
cargo build

# Release build (highly optimized for size)
cargo build --release

# Build all workspace members with all features
cargo build --all-features --workspace
```

### Formatting
```bash
# Format code (REQUIRED before committing)
cargo +nightly fmt --all

# Or use cargo-make
cargo make format
```

### Linting
```bash
# Run clippy on all targets
cargo clippy --all-features --workspace --all-targets

# CI-style (treat warnings as errors)
RUSTFLAGS="-Dwarnings" cargo clippy --all-features --workspace
```

### Testing
```bash
# Run all tests in release mode
cargo test --release --all-features --workspace

# Run tests using nextest (preferred for CI-like behavior)
cargo nextest run --profile ci --release --all-features --workspace

# Run doc tests
cargo test --release --all-features --workspace --doc

# Run a single test by name
cargo test --release --all-features -p <package> <test_name>

# Run a single test file (integration tests)
cargo test --release --all-features --test <test_file_name>

# Example: Run specific unit test
cargo test --release --all-features -p distant test_client_connect

# Example: Run specific integration test file
cargo test --release --all-features --test cli_tests

# Use cargo-make for comprehensive testing
cargo make test           # Standard tests
cargo make ci-test        # Nextest with retries
cargo make post-ci-test   # Doc tests
```

## Code Style Guidelines

### Formatting Rules (rustfmt.toml)
- **Max line width:** 100 characters
- **Line endings:** Unix (LF)
- **Indentation:** Block style
- **Field init shorthand:** Enabled
- **Import granularity:** Module level
- **Import grouping:** Std, External, Crate (in that order)
- **Impl item ordering:** Reordered for consistency

### Imports
```rust
// Standard imports (always use module-level granularity)
use std::ffi::OsString;

// External crate imports (grouped together)
use anyhow::Result;
use clap::Parser;
use serde::{Deserialize, Serialize};

// Internal crate imports
use crate::options::{DistantSubcommand, OptionsError};
use crate::{CliResult, Options};

// Module declarations after imports
mod commands;
mod common;
```

### Error Handling
```rust
// Use anyhow::Error for flexible error handling
use anyhow::{Context, Result};

// Provide context when propagating errors
let config = load_config()
    .context("Failed to load configuration file")?;

// Custom error types use derive_more
use derive_more::{Display, Error, From};

#[derive(Debug, Display, Error, From)]
pub enum CliError {
    #[display(fmt = "exit code: {}", _0)]
    Exit(#[error(not(source))] u8),
    
    #[display(fmt = "error: {}", _0)]
    Error(#[error(not(source))] anyhow::Error),
}

// Type aliases for Results
pub type CliResult = Result<(), CliError>;
```

### Async Patterns
```rust
// Use tokio runtime with async-trait
use async_trait::async_trait;
use tokio::runtime::Runtime;

#[async_trait]
pub trait AsyncHandler {
    async fn handle(&self, request: Request) -> Result<Response>;
}

// Tokio main for async binaries
#[tokio::main]
async fn main() -> Result<()> {
    // Implementation
}
```

### Types and Generics
```rust
// Prefer strong typing with clear bounds
pub fn initialize_from<I, T>(args: I) -> Result<Self, OptionsError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // Implementation
}

// Use derive macros extensively
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Fields
}
```

### Naming Conventions
- **Types:** PascalCase (e.g., `CliError`, `DistantClient`)
- **Functions/methods:** snake_case (e.g., `init_logger`, `run_command`)
- **Constants:** SCREAMING_SNAKE_CASE (e.g., `DEFAULT_TIMEOUT`)
- **Modules:** snake_case (e.g., `cli/commands`, `options/config`)
- **Private fields:** Prefix with underscore if unused (e.g., `_phantom`)

### Documentation
```rust
// Include README in lib.rs doc comments
#![doc = include_str!("../README.md")]

// Document public APIs with examples when appropriate
/// Creates a new CLI instance by parsing command-line arguments
///
/// # Errors
/// Returns `OptionsError` if argument parsing fails
pub fn initialize() -> Result<Self, OptionsError> {
    // Implementation
}
```

### Module Organization
```
src/
├── lib.rs           # Library exports, includes README docs
├── main.rs          # Entry point with platform-specific logic
├── cli.rs           # CLI initialization and routing
├── cli/
│   ├── commands/    # Command implementations
│   └── common/      # Shared CLI utilities
├── options/         # Command-line options
│   ├── common/      # Common option types
│   └── config/      # Config file structures
└── constants.rs     # Application constants
```

### Platform-Specific Code
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

### Feature Flags
```rust
// Optional dependencies with feature gates
#[cfg(feature = "libssh")]
use distant_ssh2::Ssh2Session;

// In Cargo.toml
[features]
default = ["libssh", "ssh2"]
libssh = ["distant-ssh2/libssh"]
```

## Important Patterns

1. **Workspace versioning:** Internal dependencies use exact version pinning (`version = "=0.20.0"`)
2. **Testing:** Use `rstest` for parameterized tests, `assert_fs` for filesystem tests
3. **Async runtime:** Tokio with full features
4. **Serialization:** Serde for JSON/TOML, MessagePack for protocol
5. **CLI:** Clap v4 with derive macros
6. **Logging:** Use `log` crate macros, configured via `flexi_logger`

## Before Committing

1. **Format code:** `cargo +nightly fmt --all`
2. **Run clippy:** `cargo clippy --all-features --workspace --all-targets`
3. **Run tests:** `cargo test --release --all-features --workspace`
4. **Check all workspace members:** Ensure changes work across the entire workspace

## Common Pitfalls to Avoid

- Don't commit without formatting with nightly rustfmt
- Don't introduce clippy warnings (CI treats them as errors)
- Don't break cross-platform compatibility (test Unix and Windows paths)
- Don't modify workspace dependency versions without updating all members
- Don't use outdated Rust patterns (prefer modern async/await over futures combinators)
- Don't skip `--all-features` when testing (features are part of the API contract)
