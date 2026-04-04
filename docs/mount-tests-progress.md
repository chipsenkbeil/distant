# Mount Backends — Progress Tracker

> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase A: Production Code Fixes

- [x] **A1** Fix FUSE+SSH EIO bug
  - Fixed: write_buffers Mutex held across network I/O in flush()
  - Fixed: double-slash path bug in normalize_path (use typed-path normalize/join)
  - Fixed: SFTP errors mapped to correct io::ErrorKind (sftp_io_error helper)
  - Fixed: Docker offset writes (tar-read, patch, tar-write)

- [x] **A2** Enforce readonly at RemoteFs level for all backends
  - check_writable() guard on write, create, mkdir, unlink, rmdir, rename
  - Works for NFS, FUSE, WCF, and FileProvider uniformly

- [x] **A3** Expose --read-ttl CLI option (was hardcoded to 30s)

- [x] **A4** Add FileProvider to cross-backend template
  - FP singleton via installed app at /Applications/Distant.app
  - Provisioning profiles checked into resources/macos/profiles/
  - build-macos-app.sh with CARGO_PROFILE support
  - Backup/restore of existing production install
  - 22/35 FP tests pass — 13 fail due to FP backend limitations (see A6)

- [x] **A5** Update docs/TODO.md with deferred features

- [ ] **A6** Fix FileProvider backend limitations (13 failing tests)
  - [ ] delete (rm, rmdir) — FP extension doesn't implement deleteItem
  - [ ] rename — FP extension doesn't implement renameItem
  - [ ] readonly enforcement — FP doesn't reject writes at the extension level
        (RemoteFs check_writable works but FP may not propagate the error)
  - [ ] mount-onto-file — FP mount doesn't validate mount point type
  - [ ] nonexistent remote root — FP mount doesn't fail on bad root
  - [ ] status/unmount — mount-status and unmount commands don't work with
        FP domains (is_running_in_app_bundle guard blocks CLI usage)
  - [ ] multi_mount dropping — FP domain cleanup between tests incomplete
  - These are **production code** fixes in `distant-mount/src/backend/macos_file_provider/`

---

## Phase B: Test Infrastructure Improvements

- [x] **B1** Replace fixed sleeps with polling helpers
- [x] **B2** Remove `mount_op_or_skip!` macro
- [x] **B2.5** Fix stale mount process leaks after test run
- [-] **B3** Fix all test hacks
  - [x] FRN-02, MML-03, RRT-02: done
  - [ ] MST-03: Assert exact "No mounts found" output
- [x] **B4** FileProvider test fixture in MountProcess — uses FP singleton
- [ ] **B5** Windows VM test script

---

## Phase C: Test Quality

- [-] **C1** Full cross-backend parity
  - [x] All tests work for Host, SSH, Docker, FUSE
  - [-] FileProvider: 22/35 pass, 13 fail (blocked on A6)
  - [x] Readonly enforced at RemoteFs level

- [ ] **C2** Missing test coverage (large files, cache TTL)
- [ ] **C3** Run code-validator + test-validator on all code

---

## Phase D: Documentation

- [ ] **D1** Update MANUAL_TESTING.md with final results
- [-] **D2** Final update of PRD + progress docs (this file)
- [x] **D3** Update docs/TODO.md with deferred items

---

## Singleton Test Server Infrastructure (Completed)

- [x] Host + SSH singletons (file-lock coordination, lonely shutdown)
- [x] FileProvider singleton (installs app, App Group socket, backup/restore)
- [x] `--shutdown lonely=N` on distant manager listen
- [x] Process leak fixes, daemon test rewrite

**Result:** 221/234 tests pass. Run time ~515s with FP tests.
Singletons: Host + SSH + FileProvider. After lonely timeout, only
Docker per-test manager lingers.

---

## Current State (2026-04-03)

**221/234 mount tests passing.** Breakdown:
- 199/199 non-FP tests (NFS, FUSE, Docker) — all green
- 22/35 FileProvider tests — passing (reads, creates, modifies, browse)
- 13/35 FileProvider tests — failing (backend limitations, see A6)

**Key achievements this session:**
- FUSE+SSH EIO root cause found and fixed (SFTP error mapping)
- Singleton test servers (Host, SSH, FileProvider)
- Docker offset write support
- FileProvider in cross-backend template via installed app approach
- Provisioning profiles checked into repo
- build-macos-app.sh with debug/release profile support

**Next: A6 — Fix FileProvider backend to pass all 13 failing tests**

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `BackendCtx`
- **Singletons:** `singleton.rs` — Host, SSH, FileProvider
- **Templates:** `plugin_x_mount` via rstest_reuse (in binary crate mod.rs)
- **Mount helper:** `MountProcess` with `spawn()` and `try_spawn()`
- **Seed data:** `ctx.cli_write()`, `ctx.cli_mkdir()`, `ctx.unique_dir()`
- **Verification:** `ctx.cli_read()`, `ctx.cli_exists()`
- **Polling:** `mount::wait_until_exists/content/gone` (200ms interval, 10s timeout)
- **Run:** `cargo nextest run --all-features -p distant -E 'test(mount::)'`
