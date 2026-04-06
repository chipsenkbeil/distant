# Mount Backend Fix & Test Loop

Iteratively fix production bugs, improve test infrastructure, and achieve
a fully green mount test matrix with zero workarounds.

## Context Files

Read these at the start of every iteration:

1. `docs/mount-tests-PRD.md` — architecture, phases, dependencies
2. `docs/mount-tests-progress.md` — current completion status
3. `docs/MANUAL_TESTING.md` — full test case descriptions
4. `docs/TESTING.md` — naming conventions

## Phase Order & Dependencies

```
A1 (FUSE+SSH fix) ──→ B2 (remove skip macro) ──→ C1 (cross-backend)
A2 (readonly)     ──→ C1
A3 (TTL CLI)      ──→ C2 (TTL tests)
A4 (FP template)  ──→ B4 (FP fixture) ──→ C1
A5 (TODOs)        → D3
B1 (polling)      → C1
B3 (test hacks)   → C1
B5 (Windows script) — independent
```

## Key Patterns

### Agent Usage

Use custom agents from `.claude/agents/` per CLAUDE.md:
- **rust-explorer** for investigating bugs (A1, A2)
- **rust-coder** for production fixes (A1-A4) and test infra (B1-B5)
- **code-validator** after any production code change (BLOCKING)
- **test-implementor** for test rewrites (C1-C2)
- **test-validator** after test changes (BLOCKING)

### MountProcess + Template

All tests use `#[apply(super::plugin_x_mount)]` with `skip_if_no_backend!`.
See PRD for full pattern.

### Polling Helpers (Phase B1)

Replace `wait_for_sync()` (2s sleep) with:
```rust
mount::wait_until_exists(&ctx, &path);    // polls 200ms, 10s timeout
mount::wait_until_content(&ctx, &path, expected);
mount::wait_until_gone(&ctx, &path);
```

## Iteration Protocol

### Step 1: Select Next Item
Pick the first `[ ]` item in progress.md whose dependencies are met.
`[-]` items take priority.

### Step 2: Implement
- Production fixes: use rust-explorer → rust-coder → code-validator pipeline
- Test changes: use test-implementor → test-validator pipeline
- `cargo fmt --all` + `cargo clippy --all-features --workspace --all-targets`

### Step 3: Test
```bash
cargo nextest run --all-features -p distant -E 'test(mount::)'
```

### Step 4: Update Progress
Mark `[x]` when done. Mark `[-]` with notes if partial.

### Step 5: Report
```
== Mount Fix Loop Iteration ==
Item:    A1 — Fix FUSE+SSH EIO bug
Status:  [x] Complete
Details: Root cause was X, fixed by Y
Next:    A2 — Enforce readonly on WCF + FP
```

## Rules

- **One item per iteration** (unless items are trivially small)
- **Always update progress.md**
- **Respect dependency order**
- **All tests must pass** before moving on
- **Use nextest** (not `cargo test`)
- **Commit after each phase milestone** (A complete, B complete, etc.)
- **Use custom agents** per CLAUDE.md pipeline
