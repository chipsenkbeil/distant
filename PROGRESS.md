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

### Phases E–K — Test Quality & Stability (next slice)

This batch is driven directly by the friction observed during the
Phase 0–6 rollout. See
[PRD.md § Lessons from Phase 0–6 implementation](PRD.md#lessons-from-phase-06-implementation-2026-04-07)
for the inventory of incidents that motivate each phase. Acceptance
criteria and concrete deliverables live in
[PRD.md § Plan: Test Quality & Stability](PRD.md#plan-test-quality--stability).

#### Phase E — State hygiene

- [ ] **E1** `scripts/test-mount-clean.sh` script: kills stale
      `distant manager`/`distant server`/`DistantFileProvider.appex`
      processes, removes lock/meta files under
      `$TMPDIR/distant-test-*`, bulk-removes stale FP domains via
      `distant unmount --include-all-macos-file-provider-domains`,
      and prunes `$TMPDIR/distant-test-mount-shared-*` orphans.
      Idempotent. Ships with a `--check` flag for CI dry-run.
- [ ] **E2** Build-hash validation in singleton meta files.
      `start_*` writes `git rev-parse HEAD || sha256(binary)` into
      the meta JSON; clients refuse to attach to a singleton whose
      hash doesn't match the current binary and tear it down
      (rather than silently producing "No mounts found" on a
      protocol mismatch).
- [ ] **E3** Move stale FP domain bulk cleanup from the test exit
      path to the test entry path. Today
      `cleanup_all_stale_mounts()` only runs in the no-mounts-test
      and on `Drop`; if a previous run aborted, the next run sees
      60+ accumulated CloudStorage entries and the FP discovery
      diff fails.

#### Phase F — Diagnostics & observability

- [ ] **F1** `assert_mount_status!` macro that captures full
      diagnostic context on failure: manager binary path, manager
      PID, socket path, log file tail, raw command stdout/stderr,
      JSON value (when applicable). Replaces the bare
      `assert!(stdout.contains("nfs"), "...")` pattern that hides
      the failure root cause.
- [ ] **F2** `MountSingletonHandle::diagnostic_dump(&self) -> String`
      that returns a structured snapshot (PID, socket path, lock
      file path, last 50 lines of manager log) for inclusion in
      panic messages. Wired into HLT-* test panics by default.
- [ ] **F3** Inline tail-of-log dumps in mount test panics. When a
      mount integration test fails, automatically slurp the last
      100 lines of the manager log and the server log into the
      panic message via `panic::set_hook`. Currently logs are
      written to disk and never surface unless I grep manually.

#### Phase G — Test isolation

- [ ] **G1** `MountSingletonScope::Owned` variant — explicit opt-in
      for tests that need a fresh manager+server because they
      mutate global state (kill, unmount --all, mount/unmount
      cycles). Default stays `Shared` for read-only/additive
      tests. The choice is per-test, not per-file.
- [ ] **G2** PID-locked sentinel files for singletons. Use the
      already-imported `fs4` crate to take an exclusive lock on
      the meta file when a singleton is owned, write `{pid,
      build_hash, started_at, socket}` JSON inside the lock.
      Detect stale locks (PID gone) and reclaim them safely.
- [ ] **G3** `MountTempDir` RAII helper that registers itself with
      a process-wide cleanup list and is reaped via
      `panic::set_hook` even when a test panics. Today
      `assert_fs::TempDir` cleans on `Drop` but a panic
      mid-`new_std_cmd` skips the drop and leaks the dir.

#### Phase H — Coverage gaps

- [ ] **H1** Wire format compatibility tests. Frozen JSON fixtures
      under `distant-core/src/protocol/fixtures/v0.21.0/*.json`
      cover every request/response variant. The test loads each
      fixture and asserts it round-trips through the current
      types. Catches breaking changes like the
      `MountInfo.status: String → MountStatus` flip before they
      ship. Inspired by
      [`gitoxide`'s pack-format snapshots](https://github.com/Byron/gitoxide).
- [ ] **H2** HLT-01..04 + EVT-01..02 (deferred from Phase 5).
      Needs:
      - `with_isolated_sshd` test fixture that owns a
        single-test sshd and can `kill -9` + restart it on demand.
      - `EventCapture` fixture that subscribes to the manager
        bus, buffers events into a channel, and provides
        `expect_within(timeout, predicate)` assertions.
      - HLT-01 healthy steady state, HLT-02 connection drop →
        disconnected, HLT-03 reconnect → active, HLT-04 backend
        failure → failed, EVT-01 generic subscribe, EVT-02 mount
        events on the same subscription.
- [ ] **H3** Cross-version singleton compatibility test. Build
      the binary at `master`, write its hash into a meta file,
      then connect with the current binary and assert the manager
      either accepts the request (backwards compatible) or
      refuses cleanly with a version error (not silently empty).
      Uses `cargo build --release --target-dir target/compat-test`
      with a known-good baseline tag.
- [ ] **H4** Soak / leak detection tests (gated `#[ignore]`,
      run via `cargo nextest run --run-ignored only`). Loop
      mount/unmount/list for N minutes and assert process count,
      open FD count, and `$TMPDIR/distant-test-*` file count
      stay flat. Catches the kind of leaks that produced the 60+
      orphaned CloudStorage dirs over time.
- [ ] **H5** Per-backend probe tests, one per backend, that
      simulate the backend's failure mode and assert the probe
      returns `Failed`:
      - NFS: kill the in-process NFS listener task → probe
        Failed within 1s
      - FUSE: externally `umount -f` the mount point → probe
        Failed within 1s
      - FileProvider: `removeDomain` via `NSFileProviderManager`
        directly → probe Failed within 1s
      - WCF: `CfDisconnectSyncRoot` directly → probe Failed
        within 1s
      Layered on top of Phase 4's coarse "task ended" probe once
      the granular per-backend probes land.
- [ ] **H6** Property-based round-trip tests with `proptest` for
      every protocol type
      (`MountStatus`, `Event`, `EventTopic`,
      `ManagerRequest`, `ManagerResponse`, `MountInfo`,
      `ConnectionState`). Catches edge-case serde regressions
      that hand-written round-trip tests miss. Inspired by
      [`tokio`'s use of proptest for codec testing].

#### Phase I — Test infrastructure simplification

- [ ] **I1** Typed `DistantCmd` builder in
      `distant-test-harness::cmd`. Fluent API:
      ```rust
      DistantCmd::new(ctx)
          .status()
          .show(ResourceKind::Mount)
          .format_json()
          .run()
          .expect_success()
          .json::<Vec<MountInfo>>()
      ```
      Each method maps to a real CLI flag, so subcommand typos
      (like the `manager list` and `client kill` mistakes I made
      writing HLT-05) become compile errors.
- [ ] **I2** Test fixtures for common scenarios in
      `distant-test-harness::fixtures`:
      - `MountedHost { mount_id, mount_point, ctx, _guard }` —
        `setup` connects + mounts + waits for ready, `Drop`
        unmounts.
      - `MountedSsh`, `MountedDocker` — same shape.
      - `IsolatedManager` — owns a fresh manager+server pair for
        one test, killed on drop.
      - `EventCapture` — subscribed to the bus, exposes
        `assert_eventually(predicate)`.
- [ ] **I3** Promote `ScriptedMountHandle` from
      `distant-core::net::manager::server::tests` into
      `distant-test-harness::mock`. Add sibling variants:
      - `BlockingMountHandle` — `unmount` blocks forever (tests
        timeout handling)
      - `FailingMountHandle` — `unmount` returns the configured
        error
      - `LaggyMountHandle` — `probe` sleeps for the configured
        duration (tests interval slippage)
- [ ] **I4** Faster build / iter cycle:
      - Add `[profile.dev-fast]` with
        `inherits = "dev"`, `debug = "line-tables-only"`,
        `incremental = true`, `codegen-units = 256`. Optional
        for local iteration.
      - Document `mold` (Linux) / `lld` (mac) linker setup in
        `docs/BUILDING.md`.
      - `cargo test-mount-fast` alias that uses `--profile=dev-fast`
        and only runs the affected crate's tests.

#### Phase J — CI invocation

- [ ] **J1** Tighter `[profile.mount]` in `.config/nextest.toml`:
      - Lower retry count from 5 to 2 for mount tests so flakes
        surface as failures faster.
      - Mark known-flaky tests with `#[ignore = "tracking
        #ISSUE"]`.
      - Add `--no-tests warn` so subset filters fail loudly when
        a typo selects no tests.
- [ ] **J2** `scripts/test-mount-preflight.sh`. Runs E1's cleanup
      script, verifies binaries are up-to-date (`cargo build`
      first to avoid races), warns when sshd/Docker are
      unavailable, prints the resolved test command. Documented
      in `docs/TESTING.md` as the canonical way to run mount
      tests locally.
- [ ] **J3** `scripts/test-report.sh` that parses
      `cargo nextest run ... --message-format=libtest-json` (or
      the new structured event stream) and produces a
      categorized markdown report (compilation / panic / timeout
      / flaky / leaky). Useful for CI artifact upload.

#### Phase K — Documentation & process

- [ ] **K1** `docs/TESTING.md` additions:
      - "Diagnosing flaky mount tests" recipe section
      - "Cleaning singleton state" walkthrough (links to E1)
      - "Why my test sees 'No mounts found'" troubleshooting
        section
      - Document the "preflight before mount tests" pattern
- [ ] **K2** CLAUDE.md test author checklist. One-page checklist
      covering: which fixture to use (Shared vs Owned
      singleton), how to add diagnostic context to assertions,
      when to use `proptest` vs hand-rolled cases, when to gate
      a test with `#[ignore]`.

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
