# Mount Backends — Progress Tracker

> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Active plan

The next chunk of work is **Network Resilience + Mount Health**,
which incorporates the unfinished
[chipsenkbeil/distant#288](https://github.com/chipsenkbeil/distant/pull/288)
network resilience stack and layers mount health on top.

> **The full step-by-step lives in
> [PRD.md § Plan: Network Resilience + Mount Health](PRD.md#plan-network-resilience--mount-health).**
> Refer to that section for cherry-pick targets, refactor details,
> and acceptance criteria. The checklist below is the trail of
> work in progress.

### Phase 0 — PR #288 incorporation

- [x] **0a** Move + correct PRD/PROGRESS docs, embed plan into PRD
      (commit `eb0747b`)
- [x] **0b** TCP keepalive — `TcpTransport::set_keepalive` public
      API instead of `pub(crate) use` lift (commit `b6ea29c`)
- [x] **0c** Heartbeat failure escalation
      (`max_heartbeat_failures` on `ServerConfig`, commit
      `063a1f6`)
- [x] **0d** `Plugin::reconnect` + `reconnect_strategy` with
      per-plugin `ExponentialBackoff` (commit `5e1d1ee`)
- [x] **0e** Backend health monitors — SSH + Docker self-shutdown
      via `ShutdownSender` + `ApiServerHandler::from_arc` (commit
      `1cab529`)
- [x] **0f** `ManagerConnection` `clone_connection_watcher` +
      `connection_monitor` task + `replace_client` (commit
      `ce7cccd`)
- [x] **0g** Generic `Subscribe`/`Event` protocol — addresses
      review comments 2933812110, 2933814790, 2933821911,
      2933826601. New `data/event.rs` module with `EventTopic`
      and `Event { ConnectionState, MountState }` (commit
      `52ab230`)
- [x] **0h** Manager reconnection orchestration —
      `handle_reconnection`, `NonInteractiveAuthenticator`,
      death loop, real Subscribe/Reconnect handlers (commit
      `eba8509`)
- [x] **0i** CLI: `subscribe_and_display_events`, `distant client
      reconnect`, `--no-reconnect`, `--heartbeat-interval`,
      `--max-heartbeat-failures` (commits `543f013`, `2234acb`)
- [x] **0j** Validation gate — 228/228 mount integration tests
      pass; clippy + fmt clean across the workspace; 2291
      distant-core lib tests pass

Plus an incidental fix:
- [x] `fix(test-harness)`: gate FileProvider singleton on the
      mount feature so distant-core/host/ssh/docker subset tests
      build standalone (commit `2b1a2bf`)

### Phase 1–6 — mount health on top of generic event bus

- [x] **Phase 1** `MountStatus` enum + `Event::MountState`
      (commit `ae850c5`). Wire shape:
      `{"state":"active"}` / `{"state":"failed","reason":"..."}`.
      `format_mount_status` helper for shell rendering of
      `distant status --show mount`. CLI `display_event` learns
      the new variant.
- [x] **Phase 2** `MountHandle::probe` trait extension with
      `MountProbe { Healthy, Degraded, Failed }` (commit
      `d8efb62`). Default impl returns `Healthy`.
- [x] **Phase 3** `ManagedMount` restructure + per-mount monitor
      task + kill-leak fix (commit `46caad5`).
      `info: Arc<RwLock<MountInfo>>`,
      `handle: Arc<Mutex<Option<...>>>`,
      `monitor: JoinHandle<()>`. New `monitor_mount` task
      polls every `Config::mount_health_interval` (default 5s)
      and reacts to connection state events from the broadcast
      bus. `kill(id)` now tears down mounts on the killed
      connection (latent leak fix).
- [x] **Phase 4** Backend probe implementations (commit
      `9ff6cd0`). All backends report `Failed("mount task ended")`
      via `core::MountHandle::is_alive()`. FileProvider
      additionally checks `list_file_provider_domains()` and
      returns `Degraded`/`Failed` if the OS-side domain has
      disappeared. Granular per-backend probes (NFS server task
      lift, FUSE BackgroundSession lift, WCF watcher) deferred.
- [x] **Phase 5** Tests (commit `9e8e5ea`).
      - distant-core unit tests for `MountStatus` serde,
        `probe_to_status`, `connection_state_to_mount_status`,
        `monitor_mount` (3 e2e tests with a scripted MountHandle
        test double).
      - CLI integration: HLT-05
        `kill_should_remove_mounts_owned_by_connection` —
        regression test for the kill-leak fix.
      - HLT-01..04 + EVT-01..02 (require sshd kill + connection
        drop orchestration in the integration harness) deferred
        to a follow-up.
- [-] **Phase 6** Documentation roll-up (this commit)

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
  - [x] macFUSE noappledouble/noapplexattr/nobrowse to suppress system scanning
  - [x] NFS nobrowse/noappledouble/soft/intr mount options (industry standard)
  - [x] NFS shutdown restructured: unmount before dropping listener via child task
  - [x] UNMOUNT_TIMEOUT (10s) on diskutil unmount force
  - [x] DROP_UNMOUNT_TIMEOUT (15s) on MountProcess::drop CLI call
  - [x] Skip wait_for_unmount after successful manager unmount
  - [x] Singleton mount: 22 tests share one mount per backend (16x NFS speedup)
  - [x] Docker singleton: persistent container across tests (like Host/SSH)
  - [x] FP extra metadata injection (connection_id, destination, log_level)
  - [x] FP singleton liveness uses dir check (not mount table)
  - [x] unmount_all test isolated with own HostManagerCtx
  - [x] Mount test parallelism increased from 2 to 8 threads
  - [x] Deleted redundant daemon.rs (identical to browse test)
  - [x] FP enumeration timing — wait_for_path + working set polling
        (commits `df782cd`, `5b5dcb9`, `86d794d` got the FP suite to
        37/37 and the overall total to **228/228**)

  **Phase 5: Health monitoring + connection resilience** —
  superseded by [PRD.md § Plan: Network Resilience + Mount
  Health](PRD.md#plan-network-resilience--mount-health). The new
  plan incorporates PR #288 instead of building bespoke
  infrastructure. Tracked in the "Active plan" section at the top
  of this file.
  - [ ] Periodic health check per mount (FUSE task alive, NFS socket bound,
        FP domain registered, WCF sync root registered) → Phase 4
  - [ ] Connection drop → mount status = "disconnected" → Phase 3
  - [ ] Reconnect → mount resumes → Step 0h + Phase 3
  - [ ] Permanent failure → mount status = "failed" → Phase 4
  - [ ] Generic event subscription system (incorporated from PR
        #288) → Step 0g

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

- [x] **C1** Full cross-backend parity
  - [x] All tests work for Host, SSH, Docker, FUSE
  - [x] FileProvider: 37/37 pass
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

- [x] Host + SSH + Docker singletons (file-lock coordination, lonely shutdown)
- [x] FileProvider singleton (installs app, App Group socket, backup/restore)
- [x] Docker singleton (persistent container, manager with lonely timeout)
- [x] Mount singletons: one shared mount per (backend, mount_backend) pair
- [x] `--shutdown lonely=N` on distant manager listen
- [x] Process leak fixes, daemon test rewrite

**Result:** 228/228 tests pass. Run time ~250s with 8 parallel threads.
Singletons: Host + SSH + Docker + FileProvider + mount singletons.

---

## Current State (2026-04-06)

**228/228 mount tests passing (100%).** All FP failures resolved.

| Backend     | Tests | Result   |
|-------------|-------|----------|
| Host NFS    | 37    | All pass |
| Host FUSE   | 37    | All pass |
| SSH NFS     | 37    | All pass |
| SSH FUSE    | 37    | All pass |
| Docker NFS  | 18    | All pass |
| Host FP     | 37    | All pass |
| Other       | 25    | All pass |
| **Total**   | **228** | **All pass** |

**Final FP fix series:**
- `9f0c834` — only `remove_domain_blocking` when an existing domain
  with the same ID is present (was unconditional, churned the appex).
- `05f9685` — FP unmount actually removes the macOS domain (was a
  no-op).
- `df782cd` — signal enumerator on bootstrap, wait for FP mount
  readiness.
- `5b5dcb9` — FP working set polling, configurable `poll_interval`,
  `--extra` CLI flag.
- `86d794d` — `mount::wait_for_path` helper + working set polling
  brings the FP suite to 37/37.

**Next:** see [Active plan](#active-plan) at the top of this file.

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
