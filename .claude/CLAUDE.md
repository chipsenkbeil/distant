# distant — AI Development Guide

## Project Overview

- **Language:** Rust (Edition 2024, MSRV 1.88.0)
- **Architecture:** Cargo workspace with 5 member crates
- **Type:** CLI application with client-server architecture and plugin backends
- **Main crates:** binary CLI (`distant`), core library (`distant-core`),
  Docker/SSH/host backend plugins, test harness (`distant-test-harness`)

## Key Commands

```bash
# Format (REQUIRED before committing)
cargo fmt --all

# Lint
cargo clippy --all-features --workspace --all-targets

# Lint (CI-style — required before pushing)
RUSTFLAGS="-Dwarnings" cargo clippy --all-features --workspace --all-targets

# Test all
cargo test --all-features --workspace

# Test single crate
cargo test --all-features -p <crate>

# Nextest (CI-style)
cargo nextest run --profile ci --all-features --workspace --all-targets
```

## Before Committing

1. Format, lint, and test (commands above)
2. Feature branch (`feature/<name>`) for multi-file work — never commit to `master`
3. Commit per phase / logical unit — fmt and clippy must pass before each commit
4. Never bypass GPG signing (`-c commit.gpgsign=false`, `--no-gpg-sign`) — the
   GPG key also handles SSH push auth. If signing fails, stop for the user
5. Always use `--all-features` when testing
6. Always run tests after creating or modifying test files — never assume they pass

## Anti-Patterns (Quick Reference)

1. Needless borrows in `#[cfg(windows)]` code — invisible to macOS clippy
2. Forward-slash in `PathBuf::join()` — use chained `.join()` calls
3. ssh-agent per-test — use direct key file loading
4. Mass parallel SSH/Docker tests without throttling
5. Dismissing test failures as "intermittent" without investigation
6. `xcopy /I` for single-file copies on Windows — use `copy /Y` for files
7. Skipping `--all-features` when testing
8. Outdated Rust patterns — prefer modern async/await over futures combinators
9. Modifying workspace dependency versions without updating all members

## Agent Orchestration Guide

### Available Agents

| Agent | Purpose | Triggers On |
|-------|---------|-------------|
| rust-explorer | Codebase & ecosystem research | Questions, planning, understanding code |
| rust-coder | Implementation | Writing/modifying production code |
| code-validator | Production code review | After code changes, before testing |
| test-implementor | Test writing | After code is validated, or TDD-first |
| test-validator | Test quality gating | After tests are written |

### Pipeline: Feature Implementation

1. **Explore** → understand code, find reusable utilities
2. **Implement** → write production code
3. **Validate Code** → review for quality (BLOCKING, max 3 rounds)
4. **Write Tests** → create tests at all applicable tiers
5. **Validate Tests** → review test quality (BLOCKING, max 3 rounds)
6. **Report** → summarize to user

### Pipeline: Simple Questions

Spawn only rust-explorer. No coding pipeline needed.

### Pipeline: Test-Only Changes

Explore → Write Tests → Validate Tests

### Pipeline: TDD

Explore → Write Tests First → Implement → Validate Code → Validate Tests

### Feedback Loop Rules

- Max 3 iterations per validator before escalating to user
- Validator reports issues → implementor fixes → re-validate

## General AI Workflow

1. **TDD-First Loop:** Generate test cases and minimum documentation first.
   Approve the contract before writing production code.
2. **Recursive Refinement:** Ask for critiques and alternatives instead of
   manually fixing "off" code.
3. **LSP-Context Injection:** Provide current LSP diagnostics and compiler
   errors alongside code snippets.

## Reference Documentation

- **Coding conventions:** `docs/CONVENTIONS.md`
- **Testing guide:** `docs/TESTING.md`
- **Technical debt:** `docs/TODO.md`
- **Plugin architecture:** `docs/PLUGINS.md`
- **Building & releases:** `docs/BUILDING.md`, `docs/PUBLISHING.md`

## Technical Debt & TODOs

Track in [`docs/TODO.md`](../docs/TODO.md). When resolving an item, remove it
entirely — don't leave it marked as "RESOLVED".

## Memory Bank Maintenance

- Checkpoint at session end: update CLAUDE.md, remove deprecated patterns
- Technical debt goes in `docs/TODO.md`
- Anti-pattern corrections go in the Anti-Patterns section above
- Version pinning: Rust 1.88.0+ (Edition 2024)
