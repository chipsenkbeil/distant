# distant Coding Conventions

Source of truth for coding standards and patterns. Describes approaches
abstractly — the codebase is the source of truth for current names and paths.

## Module Organization

- Library crates re-export public items at the crate root for flat imports
- Avoid deeply nested public paths — users import directly from the crate root
- Private inner modules with selective `pub use` re-exports
- Group related functionality into focused modules
- Use `mod.rs` or named module files consistently within each crate

## Abstraction Philosophy

- Prefer trait hierarchies for swappable implementations over concrete types
- Separate complex logic into dedicated structs — CLI commands translate user
  options, then delegate to the struct for actual work
- Use newtype wrappers for domain concepts (paths, IDs, keys) to prevent
  misuse at the type level
- Use builder patterns for types with many configuration options
- Use type aliases to simplify complex generic signatures and make intent clear
- Avoid premature abstraction — three similar lines is better than a one-use
  helper function

## Error Handling

- **Binary crate**: Use flexible error chaining (anyhow-style) with contextual
  messages. Every error propagation adds context describing what operation failed:
  ```rust
  let config = load_config().context("Failed to load configuration file")?;
  ```
- **Library crates**: Define situation-specific error types using derive macros
  (derive_more for Display, Error, From). No anyhow in library code:
  ```rust
  #[derive(Debug, Display, Error, From)]
  pub enum MyError {
      #[display("operation failed: {}", _0)]
      OperationFailed(#[error(not(source))] String),
  }
  ```
- Use type aliases to simplify Result types: `pub type MyResult = Result<(), MyError>;`
- Custom error types should be enums with descriptive variants
- Error messages should be helpful and actionable for end users
- No `unwrap()` in production code except where safety is documented with a
  comment explaining why it cannot fail

## Documentation Standards

- All public items in library crates MUST have doc comments
- Start with a one-line summary explaining what the item does
- Include `# Examples` section with compilable doctests for non-trivial items
- Include `# Errors` section for fallible functions
- Show typical usage and edge cases in examples
- Explain *why* to use something, not just *how*
- Do not add doc comments to `#[cfg(test)] mod tests` blocks

### Documentation Example Pattern

Good documentation follows this structure:

```rust
/// One-line summary of what this type/function does.
///
/// Longer description if needed, explaining behavior, constraints,
/// and design rationale.
///
/// # Examples
///
/// ```
/// // Show typical usage with realistic values
/// let result = my_function("example input");
/// assert_eq!(result, expected_output);
/// ```
///
/// # Errors
///
/// Returns `Err` if [describe the failure condition].
```

## Async Patterns

- Use Tokio with full features as the async runtime
- Async trait methods return pinned futures for dynamic dispatch
- Use lazy async initialization for expensive one-time setup
- Avoid blocking calls in async context — use spawn_blocking if needed
- Prefer `async fn` where the compiler allows; use pinned futures in trait
  definitions

## Naming

- Rely on module namespaces for clarity — don't redundantly prefix type names
  with the crate or module name (e.g., `Client` not `DistantClient`)
- Provide clear, meaningful names that follow established Rust ecosystem norms
- Prefix unused bindings with `_` (including phantom type markers)
- CLI options: long form for less-used options, short flags for frequent ones,
  concise documentation per option

## Import Ordering

1. `std::` imports
2. External crate imports (alphabetical)
3. Internal `crate::` imports

## Serialization

- JSON and TOML for human-readable config
- MessagePack for binary protocol
- String-based serialization for complex types via `FromStr` + `Display`
- Conditional serde support behind feature flags in library crates

## Platform-Specific Code

- Use `cfg` attributes to isolate platform-specific modules and imports
- Platform-specific dependencies use `target` cfg in `Cargo.toml`
- `#[cfg(windows)]` blocks are invisible to macOS clippy — always verify in CI
- Use chained `.join()` calls for cross-platform paths, never forward-slash
  separators in `PathBuf::join()`
- Remote operations use the project's dedicated remote path type (not `PathBuf`)

## Plugin Architecture

- Backend integrations implement the core plugin trait
- Plugins receive raw destination strings, allowing non-standard URI formats
- Keep plugin APIs flexible — pass options as generic maps where appropriate
- Each plugin interprets paths in its own context (container vs remote host)

## CLI UX Principles

- Clear, concise error messages that help users recover
- Support composability: piping, well-documented exit codes, machine-readable
  output modes
- Preserve data on failure — never silently destroy user work
- Separate responsibilities: CLI command handles option translation, dedicated
  structs perform the actual logic

## Workspace & Versioning

- All crates share workspace version via `version.workspace = true`
- Internal dependencies use exact version pinning in `[workspace.dependencies]`
- All features enabled during testing (features are part of the API contract)

## Derive Macros

Use derive macros extensively to reduce boilerplate:

- `Debug`, `Clone`, `Serialize`, `Deserialize` for data types
- `Display`, `Error`, `From` (via derive_more) for error types
- `Parser` (via clap) for CLI argument structs

## Test Conventions

- **Naming**: `<subject>_should_<behavior>` — no `test_` prefix
- **Nested modules**: `should_<behavior>` when parent module provides subject
- **Dependencies**: rstest for fixtures, assert_fs for temp files, assert_cmd
  for CLI testing
- **Assertions**: Validate content, not just success/existence
- **Platform tests**: Use `#[cfg(unix)]`/`#[cfg(windows)]`, not `#[ignore]`
- See `docs/TESTING.md` for full test infrastructure details

## Exemplar Patterns

When looking for inspiration beyond this project, study these approaches:

- **CLI design**: Clap derive patterns with clean subcommand hierarchies
- **Async composition**: Service/layer patterns for middleware
- **Type safety**: Derive macros for newtypes, functional builder patterns
- **Extensions**: Extension trait patterns to add methods to foreign types
- **API control**: Sealed trait patterns to prevent downstream implementations
- **Zero-copy**: Efficient serialization patterns for protocol types

## References

External resources that informed these conventions and the agent system design.

### Claude Code Documentation

Informed agent frontmatter format, skills system, memory configuration, and
CLAUDE.md best practices.

- https://docs.anthropic.com/en/docs/claude-code/agents
- https://docs.anthropic.com/en/docs/claude-code/skills
- https://docs.anthropic.com/en/docs/claude-code/memory
- https://docs.anthropic.com/en/docs/claude-code/claude-md

### Rust Language References

Informed coding conventions, documentation standards, and platform patterns.

- https://doc.rust-lang.org/reference/ — The Rust Reference
- https://doc.rust-lang.org/std/ — Standard Library documentation
- https://doc.rust-lang.org/cargo/ — Cargo documentation
- https://doc.rust-lang.org/nomicon/ — The Rustonomicon (advanced/unsafe patterns)
- https://rust-lang.github.io/api-guidelines/ — Rust API Guidelines (naming,
  documentation, type safety conventions)
- https://rust-unofficial.github.io/patterns/ — Rust Design Patterns
- https://rust-lang.github.io/rust-clippy/stable/index.html — Clippy lint index
- https://releases.rs/ — Rust release and feature stabilization tracking
- https://blog.rust-lang.org/ — Rust blog (edition guides, feature announcements)

### Crate Documentation (Project Dependencies)

Informed specific tool choices, error handling patterns, and testing approach.

- https://docs.rs/tokio/latest/tokio/ — Async runtime
- https://docs.rs/clap/latest/clap/ — CLI argument parsing (derive macros)
- https://docs.rs/serde/latest/serde/ — Serialization framework
- https://docs.rs/anyhow/latest/anyhow/ — Flexible error handling (binary crate)
- https://docs.rs/derive_more/latest/derive_more/ — Derive macros for Display,
  Error, From
- https://docs.rs/flexi_logger/latest/flexi_logger/ — Logging configuration
- https://docs.rs/bollard/latest/bollard/ — Docker API client
- https://docs.rs/russh/latest/russh/ — SSH protocol implementation

### Crate Documentation (Test Dependencies)

Informed test infrastructure design and assertion patterns.

- https://docs.rs/rstest/latest/rstest/ — Parameterized test fixtures
- https://docs.rs/assert_cmd/latest/assert_cmd/ — CLI command testing
- https://docs.rs/assert_fs/latest/assert_fs/ — Filesystem test fixtures
- https://docs.rs/predicates/latest/predicates/ — Assertion predicates
- https://docs.rs/test-log/latest/test_log/ — Log capture in tests
- https://docs.rs/expectrl/latest/expectrl/ — PTY-based terminal testing

### Crate Ecosystem (Exemplar Patterns)

Influenced the "Exemplar Patterns" section — studied for architectural and
API design inspiration.

- https://docs.rs/tower/latest/tower/ — Service/layer middleware composition
- https://docs.rs/hyper/latest/hyper/ — Extension trait patterns for HTTP types
- https://docs.rs/typed-path/latest/typed_path/ — Documentation style exemplar
  (the original CLAUDE.md documentation examples were modeled on this crate's
  doc comments)

### General Ecosystem Tools

Used by agents for crate discovery and documentation lookup.

- https://docs.rs/ — Crate API documentation hosting
- https://crates.io/ — Rust package registry
- https://lib.rs/ — Alternative crate discovery and categorization
