# Mount Backends — Progress Tracker

> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase A: Production Code Fixes

- [x] **A1** Fix FUSE+SSH EIO bug
  - Fixed: write_buffers Mutex held across network I/O in flush()
  - Fixed: double-slash path bug in normalize_path (use typed-path normalize/join)
  - Fixed: SFTP errors mapped to correct io::ErrorKind (sftp_io_error helper)
    Root cause: all SFTP errors were ErrorKind::Other → mapped to EIO by FUSE.
    macFUSE got EIO from lookup (instead of ENOENT) and refused to call create().
  - Fixed: Docker offset writes (tar-read, patch, tar-write) — was returning
    Unsupported, breaking mount append operations
  - Result: 199/199 pass

- [ ] **A2** Enforce readonly on WCF + FileProvider
  - Investigate native readonly support in Cloud Filter API and NSFileProviderDomain
  - If no native support, enforce at Rust callback level

- [ ] **A3** Expose ALL cache TTLs via CLI
  - `--read-ttl <SECS>` (currently hardcoded to 30s)
  - `--fuse-entry-ttl <SECS>` (FUSE kernel TTL, currently 1s)
  - `--mount-option KEY=VALUE` for backend-specific options

- [ ] **A4** Add FileProvider back to cross-backend template
  - MountProcess needs FP-aware spawn (build .app bundle, spawn bundled binary)
  - Domain cleanup on drop (unmount --all via bundled binary)
  - Mount point detection: `~/Library/CloudStorage/Distant-*`

- [x] **A5** Update docs/TODO.md with deferred features
  - Updated Issue #145 with remaining mount work items
  - Added TD-0 for singleton sshd/Docker cleanup

---

## Phase B: Test Infrastructure Improvements

- [x] **B1** Replace fixed sleeps with polling helpers
  - wait_until_exists, wait_until_content, wait_until_gone in mount.rs
  - wait_until_content shows actual vs expected on timeout

- [x] **B2** Remove `mount_op_or_skip!` macro
  - Removed — all ssh_fuse write tests pass with SFTP error mapping fix

- [x] **B2.5** Fix stale mount process leaks after test run
  - daemon.rs: rewritten to use MountProcess (no orphaned re-exec children)
  - remote_root/edge_cases: replaced catch_unwind with try_spawn (panic=abort safe)
  - multi_mount: replaced catch_unwind with try_spawn
  - try_spawn returns Err on all failure paths with proper child cleanup
  - Result: singleton managers/server auto-exit via lonely=10 after tests finish.
    Only Docker per-test manager lingers (Docker doesn't use singletons yet).

- [-] **B3** Fix all test hacks
  - [x] FRN-02: Cross-dir rename asserts success (no graceful skip)
  - [x] MML-03: Same-root-twice uses try_spawn, accepts Ok or Err
  - [x] RRT-02: Nonexistent root uses try_spawn, accepts Err (NFS) or empty dir (FUSE)
  - [ ] MST-03: Assert exact "No mounts found" output

- [ ] **B4** FileProvider test fixture in MountProcess (depends on A4)

- [ ] **B5** Windows VM test script

---

## Phase C: Test Quality

- [-] **C1** Full cross-backend parity (depends on A2, A4)
  - [x] All tests work for Host, SSH, Docker, FUSE backends
  - [x] No backend-specific workarounds or EIO skips
  - [ ] FileProvider not yet in cross-backend template
  - [ ] Readonly not yet enforced on WCF/FP

- [ ] **C2** Missing test coverage
  - Large files (1MB+)
  - Cache TTL behavior tests (depends on A3)

- [ ] **C3** Run code-validator + test-validator on all code

---

## Phase D: Documentation

- [ ] **D1** Update MANUAL_TESTING.md with final results
- [-] **D2** Final update of PRD + progress docs (this file)
- [x] **D3** Update docs/TODO.md with deferred items

---

## Singleton Test Server Infrastructure (Completed)

- [x] Add `fs4`, `serde` dependencies to test harness
- [x] Add `--shutdown lonely=N` to `distant manager listen`
- [x] Create `singleton.rs` with file-lock coordination, stale cleanup
- [x] Add `owns_processes` + `_lock_file` to context types, gate Drop
- [x] Wire `ctx_for_backend()` to use singleton servers
- [x] Detach singleton processes, adjust nextest leak-timeout for mount tests
- [x] Fix daemon test leaks (use MountProcess, not manual daemon spawn)

**Result:** 199/199 tests pass. Run time ~265s (down from 595s). After
tests finish + 15s lonely timeout, only 1 Docker per-test manager remains
(Docker singleton support is future work). All other processes auto-exit.

---

## Current State (2026-04-02)

**199/199 mount tests passing.** Key improvements this session:
- FUSE+SSH EIO fully resolved (SFTP error mapping root cause)
- Docker offset write support added (was returning Unsupported)
- Singleton test servers reduce process count from hundreds to ~5
- All catch_unwind usage replaced with try_spawn (panic=abort safe)
- Stale process leaks fixed — auto-cleanup via lonely timeout
- 4 new Docker offset write integration tests

**Remaining work:**
- A2: Readonly enforcement on WCF/FileProvider
- A3: TTL CLI exposure
- A4: FileProvider back in cross-backend template
- B5: Windows VM test script
- C2: Large file + cache TTL tests
- C3: Full code/test validation pass
- Docker singleton support (eliminate last lingering process)

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `BackendCtx`
- **Singletons:** `singleton.rs` — file-lock-based shared servers (Host, SSH)
- **Templates:** `plugin_x_mount` via rstest_reuse (in binary crate mod.rs)
- **Mount helper:** `MountProcess` with `spawn()` and `try_spawn()`
- **Seed data:** `ctx.cli_write()`, `ctx.cli_mkdir()`, `ctx.unique_dir()`
- **Verification:** `ctx.cli_read()`, `ctx.cli_exists()`
- **Polling:** `mount::wait_until_exists/content/gone` (200ms interval, 10s timeout)
- **Run:** `cargo nextest run --all-features -p distant -E 'test(mount::)'`
