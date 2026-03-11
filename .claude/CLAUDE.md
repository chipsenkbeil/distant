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

# Lint (warnings are denied via workspace lints — no RUSTFLAGS needed)
cargo clippy --all-features --workspace --all-targets

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
10. `SftpSession::new()` uses russh-sftp's 10s default — always use `new_opts` with the crate's unified SSH timeout constant
11. Separator comments in tests (`// --- section ---`) — use flat test names instead
12. Nested test modules with `_tests` suffix — flatten with subject prefix
13. Inline type references when imports are available — import types at module top
14. Module doc comments referencing implementation details — describe purpose, not provenance

## Agent Orchestration Guide

### Agent Selection Rules

- **Always prefer custom agents** from `.claude/agents/` over builtin equivalents.
  Custom agents have project-specific skills, LSP access, and conventions that
  builtin agents lack (CLAUDE.md does NOT propagate into subagents).
- **`rust-explorer`** over `Explore` for all codebase research.
- **`rust-coder`** over generic implementation for all production code changes.
- **`code-validator`** is mandatory after production code changes (BLOCKING).
- **`test-validator`** is mandatory after test code changes (BLOCKING).
- Use builtin `general-purpose` only for tasks needing tools custom agents lack
  (e.g., `gh` CLI for CI log fetching).
- Use builtin `Plan` for design — no custom planning agent exists.

### Available Agents

| Agent | Purpose | Triggers On |
|-------|---------|-------------|
| rust-explorer | Codebase & ecosystem research | Questions, planning, understanding code |
| rust-coder | Implementation | Writing/modifying production code |
| code-validator | Production code review | After code changes, before testing |
| test-implementor | Test writing | After code is validated, or TDD-first |
| test-validator | Test quality gating | After tests are written |

### Pipeline: Feature Implementation

1. **rust-explorer** → understand code, find reusable utilities
2. **rust-coder** → write production code
3. **code-validator** → review for quality (BLOCKING, max 3 rounds)
4. **test-implementor** → create tests at all applicable tiers
5. **test-validator** → review test quality (BLOCKING, max 3 rounds)
6. **Report** → summarize to user

### Pipeline: Simple Questions

Spawn only **rust-explorer**. No coding pipeline needed.

### Pipeline: Test-Only Changes

**rust-explorer** → **test-implementor** → **test-validator**

### Pipeline: TDD

**rust-explorer** → **test-implementor** → **rust-coder** → **code-validator** → **test-validator**

### Feedback Loop Rules

- Max 3 iterations per validator before escalating to user
- Validator reports issues → implementor fixes → re-validate

### Plan-Mode Requirements

Every plan MUST begin with an **Agent Usage** section that declares:
1. Which local agents from `.claude/agents/` will be used
2. The execution order (pipeline) they will follow
3. Justification for any skipped pipeline stages

Rules:
- Always prefer local agents (`rust-explorer`, `rust-coder`,
  `code-validator`, `test-implementor`, `test-validator`) over builtin
  equivalents (`Explore`, `Plan`, `general-purpose`)
- Use `rust-explorer` in place of the builtin `Explore` agent for all
  codebase research
- Follow the standard pipeline order: **rust-explorer** → **rust-coder** →
  **code-validator** → **test-implementor** → **test-validator**
- Stages may be skipped with justification (e.g., "no test changes needed")
- Use builtin `Plan` only for design phase (no local planning agent exists)
- Use builtin `general-purpose` only when no local agent has the required
  tools (e.g., `gh` CLI)

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
