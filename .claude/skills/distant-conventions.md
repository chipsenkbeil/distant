---
name: distant-conventions
description: >
  Coding conventions, patterns, and quality standards for the distant Rust
  project. Loaded by agents to ensure consistent code quality.
---

# distant Conventions (Quick Reference)

Read `docs/CONVENTIONS.md` for the full reference. Key highlights:

## Module Organization
- Library crates re-export public items at the crate root for flat imports
- Avoid deeply nested public paths — users import from the crate root
- Private inner modules with selective `pub use` re-exports

## Error Handling
- **Binary crate**: Flexible error chaining with context on every propagation
- **Library crates**: Situation-specific error types using derive macros
- No `unwrap()` in production code without a safety comment

## Documentation
- All public items in library crates MUST have doc comments
- Start with one-line summary, add `# Examples` and `# Errors` sections
- No doc comments on `#[cfg(test)] mod tests` blocks

## Test Naming
- `<subject>_should_<behavior>` — never use `test_` prefix
- Nested modules: `should_<behavior>` (parent module provides subject)
- See `docs/TESTING.md` for test infrastructure, fixtures, and tiers

## Import Order
1. `std::` imports
2. External crate imports (alphabetical)
3. Internal `crate::` imports

## Platform Gotchas
- `#[cfg(windows)]` blocks invisible to macOS clippy — verify in CI
- Use chained `.join()` for paths, never forward-slash in `PathBuf::join()`
- Remote operations use the project's dedicated remote path type

## Abstraction Preferences
- Trait hierarchies for swappable implementations
- Newtype wrappers for domain concepts
- Builder patterns for multi-field configuration
- Dedicated structs for complex logic — CLI commands delegate to them
- Type aliases for complex generic signatures
- No premature abstraction — three similar lines beats a one-use helper

## Workspace
- All crates share workspace version
- Internal deps use exact version pinning
- All features enabled during testing

## Full Reference
- Coding conventions: `docs/CONVENTIONS.md`
- Test infrastructure: `docs/TESTING.md`
- Technical debt: `docs/TODO.md`
