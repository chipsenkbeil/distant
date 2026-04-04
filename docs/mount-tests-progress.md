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

- [-] **A6** Fix FileProvider backend — 9 tests need fixes (no skips allowed)
  Production fixes:
  - [x] delete handler — returns NSError on lookup failure (was silent success)
  - [x] rename handler — reads changedFields, performs fs.rename()
  - [x] readonly in metadata — CLI persists flag, bootstrap reads it
  - [x] canonicalize remote root — resolves symlinks at mount time
  - [ ] readonly capabilities — set NSFileProviderItemCapabilities to exclude
        AllowsWriting/AllowsDeleting when readonly=true (macOS rejects writes
        at OS level before they reach the extension)
  - [ ] per-domain unmount — store domain ID in MountProcess, unmount single
        domain on Drop instead of unmount --all
  Test fixes (FP-specific logic in test, not skips):
  - [ ] rmdir: use remove_dir_all for FP (hidden metadata in dirs)
  - [ ] unmount_by_path: use MountProcess for FP (no raw spawn)
  - [ ] unmount_all: use MountProcess for FP, remove skip
  - [ ] mount_onto_file: for FP, assert nonexistent remote root fails instead
  - [ ] status tests: run mount-status via installed binary for FP
  - [ ] multi_mount drop: depends on per-domain unmount production fix

- [ ] **A7** Manager-owned mount lifecycle (future architecture)
  Move mount lifecycle into the manager process:
  - Manager spawns mounts as internal tokio tasks (in-process)
  - FUSE: spawn_blocking with dedicated thread pool
  - NFS: async in tokio runtime
  - FileProvider: domain registration, manager tracks domain IDs
  - Windows Cloud Files: spawn_blocking for COM callbacks
  - New ManagerRequest variants: Mount, Unmount, ListMounts
  - `distant mount` becomes async (no --foreground)
  - `distant status` shows connections + mounts
  - Connection drop: keep mount alive, attempt reconnect, surface status
  - Manager shutdown: unmount everything (future: --persist-mounts flag)
  - Windows testing via ssh windows-vm + rsync + cargo nextest

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

## Current State (2026-04-04)

**~227/234 mount tests passing.** Breakdown:
- 199/199 non-FP tests (NFS, FUSE, Docker) — all green
- ~32/38 FileProvider tests pass (reads, creates, deletes, renames, browse)
- ~6 FP tests have skips that must be converted to FP-specific test logic
- 3 FP tests fail due to production gaps (readonly caps, per-domain unmount, rmdir)

**Key achievements:**
- FUSE+SSH EIO root cause: SFTP error mapping
- Singleton test servers: Host, SSH, FileProvider
- Docker offset write support
- FileProvider in cross-backend template via installed app + provisioning profiles
- Remote root canonicalization (resolves /var → /private/var symlinks)
- FP delete handler and rename handler fixed

**Next:**
- A6: Remove all FP test skips — add FP-specific test logic instead
- A6: Readonly capabilities, per-domain unmount production fixes
- A7: Manager-owned mount lifecycle (future architecture)

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
