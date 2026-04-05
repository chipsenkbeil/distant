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

- [x] **A6** Fix FileProvider backend — all 38 FP tests pass, zero skips
  Production fixes:
  - [x] delete handler — returns NSError on lookup failure
  - [x] rename handler — reads changedFields, performs fs.rename()
  - [x] readonly in metadata — CLI persists flag, bootstrap reads it
  - [x] canonicalize remote root — resolves symlinks at mount time
  - [x] readonly fileSystemFlags — excludes UserWritable for readonly mounts,
        macOS rejects writes at POSIX level (EACCES)
  - [x] readonly capabilities — excludes AllowsWriting/AllowsDeleting
  - [x] per-mount unmount — Drop passes CloudStorage path instead of --all
  Test fixes (FP-specific logic in test, no skips):
  - [x] rmdir: uses remove_dir_all for FP (hidden metadata in dirs)
  - [x] unmount_by_path: uses MountProcess + installed binary for FP
  - [x] unmount_all: same
  - [x] mount_onto_file: FP asserts nonexistent remote root fails
  - [x] status tests: runs mount-status via installed binary for FP
  - [x] multi_mount drop: per-mount unmount via CloudStorage path

- [-] **A7** Manager-owned mount lifecycle (6 phases)

  **Architecture:** Manager owns mount lifecycle via mount plugins.
  distant-core does NOT depend on distant-mount — uses generic types
  (Map for config, String for backend). Mount plugins register backends
  (NFS, FUSE, macOS FileProvider, Windows Cloud Files) similar to how
  connection plugins register schemes (host, ssh, docker).

  **Phase 1: Protocol + unified List** ✅
  - [x] MountConfig, CacheConfig, MountInfo in distant-core protocol
  - [x] ResourceKind enum (Connection, Tunnel, Mount) for List filtering
  - [x] MountPlugin + MountHandle traits in distant-core::plugin
  - [x] Mount/Unmount/Mounted/Unmounted request/response variants
  - [x] Unified List with resources filter (replaces separate listing)
  - [x] `distant mount-status` removed (clean break for 0.21.0)
  - [x] `distant status --show mount` implemented

  **Phase 2: Mount plugin implementations** ✅
  - [x] NfsMountPlugin — in-process NFS server + os_mount via spawn_blocking
  - [x] FuseMountPlugin — fuser::spawn_mount2 via spawn_blocking
  - [x] FileProviderMountPlugin — register_domain + detached handle
  - [x] CloudFilesMountPlugin — mount + connection guard
  - [x] MountHandleWrapper bridges concrete handle to trait (Mutex for Sync)
  - [x] All plugin exports top-level from distant-mount

  **Phase 3: Manager mount/unmount handlers** ✅
  - [x] Mount handler: validates plugin, opens InternalRawChannel, calls
        plugin.mount(), stores ManagedMount
  - [x] Unmount handler: removes from map (drops lock), calls
        handle.unmount(), closes manager_channel
  - [x] Mount IDs via rand::random::<u32>() (same as tunnel IDs)
  - [x] Mount plugins registered in manager Config (build_mount_plugin_map)

  **Phase 4: CLI transition + test updates** ✅
  - [x] `distant mount` sends Mount request, prints result, exits immediately
  - [x] `distant unmount` interactive selection (follows Kill pattern)
  - [x] `distant unmount <id>` / `distant unmount --all`
  - [x] Removed --foreground, daemonization wrapper, mount_with_backend()
  - [x] MountProcess test harness: cmd.output(), parse mount ID, unmount via manager
  - [x] Status/unmount tests rewritten for manager-based flow
  - [x] mount_point bug fixed (plugins return actual path, not empty string)
  - [x] unmount_path converted to async (tokio::process::Command)
  - [x] start_manager_daemon converted to async
  - [x] macFUSE noappledouble/noapplexattr to suppress system scanning
  - [x] NFS unmount ordering fix (unmount before dropping listener)
  - [x] Skip wait_for_unmount after successful manager unmount

  **Phase 5: Health monitoring + connection resilience**
  - [ ] Periodic health check per mount (FUSE task alive, NFS socket bound,
        FP domain registered, WCF sync root registered)
  - [ ] Connection drop → mount status = "disconnected"
  - [ ] Reconnect → mount resumes
  - [ ] Permanent failure → mount status = "failed"

  **Phase 6: Process count audit**
  - [ ] Audit during full test run: expect ~5 distant processes + FP appex
  - [ ] Windows Cloud Files testing via ssh windows-vm + rsync
  - [ ] ~30+ FP appex processes observed today — verify reduction

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

**A7 Phase 4 complete.** Mount lifecycle owned by manager. CLI sends
mount/unmount requests to manager instead of spawning foreground processes.

**Key changes in A7 Phases 1-4:**
- MountConfig, MountPlugin, MountHandle traits in distant-core
- 4 mount plugin implementations (NFS, FUSE, FileProvider, CloudFiles)
- Manager handles Mount/Unmount requests via InternalRawChannel
- CLI mount exits immediately (no foreground, no daemonization)
- CLI unmount by ID with interactive selection (follows Kill pattern)
- `distant status --show mount` for mount listing
- All blocking OS commands converted to async (tokio::process::Command)
- macFUSE noappledouble/noapplexattr suppresses Spotlight CPU spike
- Test harness updated: MountProcess parses mount ID, unmounts via manager
- Status/unmount tests rewritten for new flow

**Test results:** NFS host tests passing. FUSE host tests passing.
Full cross-backend validation in progress.

**Next: A7 Phase 5 — Health monitoring + connection resilience**

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
