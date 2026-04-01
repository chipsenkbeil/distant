# Mount Backends — Progress Tracker

> Auto-updated by the `/mount-test-loop` command.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase A: Production Code Fixes

- [x] **A1** Fix FUSE+SSH EIO bug
  - Root cause: write_buffers Mutex held across network I/O in flush()
  - Fix: Extract data under lock, release lock, then do network I/O
  - Also: added warn!() logging to io_error_to_errno + more errno mappings
  - All ssh_fuse tests now pass (198/199, 1 docker_nfs sync timing → B1)
  - Unlocks: B2 (remove mount_op_or_skip macro)

- [ ] **A2** Enforce readonly on WCF + FileProvider
  - Investigate native readonly support in Cloud Filter API and NSFileProviderDomain
  - If no native support, enforce at Rust callback level
  - Unlocks: C1 (readonly tests work for all backends)

- [ ] **A3** Expose ALL cache TTLs via CLI
  - `--read-ttl <SECS>` (currently hardcoded to 30s)
  - `--fuse-entry-ttl <SECS>` (FUSE kernel TTL, currently 1s)
  - `--mount-option KEY=VALUE` for backend-specific options
  - Unlocks: C2 (TTL behavior tests)

- [ ] **A4** Add FileProvider back to cross-backend template
  - MountProcess needs FP-aware spawn (build .app bundle, spawn bundled binary)
  - Domain cleanup on drop (unmount --all via bundled binary)
  - Mount point detection: `~/Library/CloudStorage/Distant-*`
  - Unlocks: B4, C1

- [ ] **A5** Update docs/TODO.md with deferred features
  - setattr (pending distant protocol), symlinks, hard links
  - File locking, extended attributes, large file streaming

---

## Phase B: Test Infrastructure Improvements

- [ ] **B1** Replace fixed sleeps with polling helpers
  - `wait_until_exists(ctx, path)` — polls every 200ms, 10s timeout
  - `wait_until_content(ctx, path, expected)` — same
  - `wait_until_gone(ctx, path)` — same
  - Replace `wait_for_sync()` (2s sleep) and 50ms sleep in rapid test

- [ ] **B2** Remove `mount_op_or_skip!` macro (depends on A1)
  - Replace all uses with `.unwrap_or_else(|e| panic!(...))`
  - All write operations must succeed for all backends

- [ ] **B3** Fix all test hacks
  - FRN-02: Cross-dir rename must assert success (not graceful skip)
  - MML-03: Same-root-twice must define + assert expected behavior
  - RRT-02: Nonexistent root must assert specific error
  - MST-03: Assert exact "No mounts found" output

- [ ] **B4** FileProvider test fixture in MountProcess (depends on A4)
  - build_test_app_bundle() called automatically
  - Bundled binary spawned for FP mounts
  - Container + socket symlink setup
  - Domain cleanup on drop

- [ ] **B5** Windows VM test script
  - `scripts/test-windows-mount.sh`
  - rsync code → build → nextest on windows-vm
  - WCF cases compile-gated to `target_os = "windows"`

---

## Phase C: Test Quality

- [ ] **C1** Full cross-backend parity (depends on A1, A2, A4)
  - Every test works for ALL backends in template
  - No backend-specific workarounds or exceptions
  - FileProvider uses MountProcess abstraction seamlessly

- [ ] **C2** Missing test coverage
  - Large files (1MB+)
  - Docker+NFS combination verified
  - Cache TTL behavior tests (depends on A3)

- [ ] **C3** Run code-validator + test-validator on all code

---

## Phase D: Documentation

- [ ] **D1** Update MANUAL_TESTING.md with final results
- [ ] **D2** Final update of PRD + progress docs
- [ ] **D3** Update docs/TODO.md with deferred items (same as A5)

---

## Prior Work (Completed in Previous Session)

These phases are complete and form the baseline:

- [x] **P1** Harness + Templates — mount feature, rstest_reuse, MountProcess, templates
- [x] **P2** Core Read Tests — browse, file_read, subdirectory (15/18 pass each, 3 FP skip)
- [x] **P3** Write Tests — file_create, file_delete, file_rename, file_modify, directory_ops
- [x] **P4** Mount Management — readonly, remote_root, multi_mount, status, unmount
- [x] **P5** Edge Cases + Daemon + Backend-Specific — all implemented

**Current state:** 199 tests all passing, but with workarounds:
- `mount_op_or_skip!` hides FUSE+SSH EIO failures
- FileProvider excluded from cross-backend template
- Fixed 2s sleeps for sync verification
- Some tests gracefully skip on certain backends

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `BackendCtx`
- **Templates:** `plugin_x_mount` via rstest_reuse (in binary crate mod.rs)
- **Mount helper:** `MountProcess` in harness mount module
- **Seed data:** `ctx.cli_write()`, `ctx.cli_mkdir()`, `ctx.unique_dir()`
- **Verification:** `ctx.cli_read()`, `ctx.cli_exists()`
- **Run:** `cargo nextest run --all-features -p distant -E 'test(mount::)'`
