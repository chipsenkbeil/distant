# Mount Backends — Progress Tracker

> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Active plan

The next chunk of work is **Network Resilience + Mount Health**,
which incorporates the unfinished
[chipsenkbeil/distant#288](https://github.com/chipsenkbeil/distant/pull/288)
network resilience stack and layers mount health on top.

### Interlude: wire-protocol error visibility (2026-04-09)

A small, targeted slice that came out of rescoping the original Phase
E+F (wire-format hardening + schema-hash-in-singleton-path). The user
rejected those as solving a non-problem and redirected to an actual
pain point: production deserialize failures surfacing as
`Deserialize failed: data did not match any variant of untagged enum
Msg`, with no indication of which type failed, no raw payload, and
the buried error gated behind `--log-level debug`.

- [x] **Sub-phase 1** Custom `Deserialize for Msg<T>` that dispatches
      via `deserialize_any` + Visitor (`visit_seq` → `Batch`,
      `visit_map` → `Single`) and forwards the real inner error from
      `T::deserialize` unchanged, eliminating the untagged-enum
      collapse. Narrows `Msg<T>` to map/seq payloads (the only shape
      used in production). 8 new `failure_paths` tests (commit
      `cb4ca00`).
- [x] **Sub-phase 2** Enrich `deserialize_from_slice` error with
      `std::any::type_name::<T>()` and slice length so every
      downstream caller inherits the context (commit `4629160`).
- [x] **Sub-phase 3** New `utils::hex_preview` + `HEX_PREVIEW_BYTES`
      helper (lowercase hex via `hex::encode`, binary-safe). Rewrite
      both decode-error arms of the server receive loop at
      `net/server/connection.rs:538-577` to always log at `error!`
      with byte length + hex preview; drop the `log_enabled!(Debug)`
      gate and the lossy `String::from_utf8_lossy` dump. 5 new
      `hex_preview` tests (commit `7198146`).
- [x] **Sub-phase 4** Same treatment for the client receive path at
      `net/client/channel.rs::map_to_typed_mailbox` (commit
      `6c84af0`).

After this slice, an `info`-level log of a failing decode emits one
line with the target type, byte length, hex preview, and the actual
inner deserialize error — no debug/trace rerun required. 15 new unit
tests total; `distant-core` lib passes 2306/2306.

The Phase E+F plan from PRD.md is explicitly **dropped** — the
visibility improvements make unknown-variant failures self-diagnosing,
and the user confirmed the singleton-path mismatch isn't a real
pain point.

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


### Phases E–L — Test Quality & Stability (revised next slice)

**Revised 2026-04-07** after ~30 minutes of dedicated research
into nextest internals, ctor/dtor crates, process supervision
patterns, and the actual harness code. The earlier draft of
Phases E–K (cleanup scripts, build-hash validation, isolation
opt-in) tried to *patch* the singleton-for-everything model.
**The singleton model itself is the bug.**

Replace it with:

1. **Per-test ephemeral fixtures for the 80% case** (host, ssh,
   docker, mount manager) — 37 of 39 mount tests don't need a
   singleton; they only got one because spawning a fresh
   manager+server per test was slow. With `command-group` for
   reliable Drop cleanup + `pdeathsig`/kqueue for SIGKILL
   coverage, the per-test cost is ~100ms — 5 seconds of extra
   wall-clock for the whole suite, in exchange for eliminating
   every cleanup failure mode.
2. **A tiny Ryuk-style sidecar reaper for the FP appex** (the
   one true singleton — macOS allows exactly one File Provider
   extension instance per bundle ID per machine). Connection
   lease lifecycle, schema-hash in socket path, self-heals
   stale state on startup.
3. **Schema-hash baked into singleton paths** so binaries from
   different wire formats automatically use different paths.
   Wire-format mismatch becomes structurally impossible — no
   silent "No mounts found", no manual cleanup.

See
[PRD.md § Test Architecture Today](PRD.md#test-architecture-today-2026-04-07)
for the diagrams of the current architecture and its failure
modes. See
[PRD.md § Plan: Test Quality & Stability](PRD.md#plan-test-quality--stability-revised-2026-04-07)
for the full revised plan with goals, deliverables, acceptance
criteria, and the explicit list of what's DROPPED from the old
draft (cleanup scripts, build-hash sentinels, owned-singleton
opt-in, etc).

#### Phase E — Wire-format hardening (~50 LOC)

- [ ] **E1** `#[serde(other)]` fallback variants on every wire
      enum in `distant-core/src/net/manager/data/{request,response,event}.rs`
      and `distant-core/src/protocol/**`. Converts "manager
      silently rejects request" into "manager sees `Unknown`
      and returns a typed error".
- [ ] **E2** Compile-time `WIRE_SCHEMA_HASH` constant computed
      via `const_fnv1a_hash` over the textual representation of
      the wire-type module files. Used by Phase F.

#### Phase F — Schema-hash-in-singleton-path (~30 LOC)

- [ ] **F1** Bake `schema_hash_hex()` into
      `singleton::base_path()` and the FP reaper socket path so
      stale singletons from old binaries automatically go to
      different paths and self-terminate via lonely shutdown.
      The single highest-impact change in the entire plan.

#### Phase G — FP reaper sidecar (~400 LOC)

- [ ] **G1** New `distant-test-reaper` workspace binary at
      `distant-test-harness/src/bin/distant-test-reaper.rs`.
      Listens on `/tmp/distant-test-reaper-<schema>.sock`,
      uses connection-presence as the lease signal, lingers
      5s after the last lease disconnects before tearing down
      the FP fixture and exiting. Self-heals stale sockets and
      orphan PIDs on startup.
- [ ] **G2** `FpFixtureLease` test-side struct in
      `distant-test-harness::fixtures::fp`. `acquire()`
      connects to the reaper socket (fork-execs the reaper
      binary if not running) and parks the connection for the
      lifetime of the struct. `Drop` closes the connection.

#### Phase H — Ephemeral host/ssh/docker fixtures (~600 LOC)

- [ ] **H1** `MountedHost` fixture in
      `distant-test-harness::fixtures::host`. Spawns its own
      manager + server via `command-group::group_spawn`, owns
      the temp dir + socket file, killpg's the entire process
      group on Drop.
- [ ] **H2** `MountedSsh` fixture. Spawns its own sshd via the
      existing `Sshd::spawn` (no longer leaked), its own
      manager via `command-group`, kills both on Drop.
- [ ] **H3** `MountedDocker` fixture. Spawns its own container
      with `auto_remove = true`, its own manager via
      `command-group`, kills both on Drop.
- [ ] **H4** `--watch-parent <PID>` flag on `distant manager`
      and `distant server` (production code, not test-only).
      On Linux this is a no-op (`pdeathsig` handles it via
      `pre_exec`); on macOS spawns an internal kqueue thread
      that watches the given PID for `EVFILT_PROC | NOTE_EXIT`
      and exits on parent death. Closes the SIGKILL hole.
- [ ] **H5** Migrate the 37 affected tests to the new
      fixtures. Test bodies are mostly unchanged; only the
      setup line swaps from `let ctx = skip_if_no_backend!(...)`
      to `let host = MountedHost::start()?;`. The two FP tests
      use `let fp = FpFixtureLease::acquire()?;`. The two
      already-isolated tests (HLT-05, unmount::all) just
      rename `HostManagerCtx::start()` to `MountedHost::start()`.

#### Phase I — Test infrastructure simplification (~500 LOC)

- [ ] **I1** Typed `DistantCmd` builder in
      `distant-test-harness::cmd`. Compile-time CLI typo
      catching (the HLT-05 test had two CLI typos on first
      attempt; this would have caught them).
- [ ] **I2** `assert_mount_status!` macro that captures full
      diagnostic context on failure (binary path, PID, socket,
      command, stdout/stderr, log file tail).
- [ ] **I3** Promote `ScriptedMountHandle` from
      `distant-core::net::manager::server::tests` into
      `distant-test-harness::mock`. Add `BlockingMountHandle`,
      `FailingMountHandle`, `LaggyMountHandle` siblings.
- [ ] **I4** `[profile.dev-fast]` in workspace `Cargo.toml`
      with `mold`/`lld` linker setup documented in
      `docs/BUILDING.md`.
- [ ] **I5** Process-wide `panic::set_hook` that, on test
      panic, looks up the active fixture's log files via a
      thread-local registry and prints the last 100 lines of
      each.

#### Phase J — Coverage gaps (~800 LOC)

- [ ] **J1** Frozen wire-format JSON fixtures in
      `distant-core/src/protocol/fixtures/`. One round-trip
      test per request/response/event variant.
- [ ] **J2** HLT-01..04 + EVT-01..02 (deferred from Phase 5).
      Now trivial because each test owns its own
      `MountedSsh` and can `kill -9` the sshd directly.
- [ ] **J3** Soak tests gated `#[ignore]` that loop 100
      mount/unmount cycles per backend and assert process /
      FD / tempfile counts stay flat.
- [ ] **J4** Per-backend probe tests once granular probes
      land (deferred from Phase 4 of Network Resilience).
- [ ] **J5** Property-based round-trip tests with `proptest`
      over every wire enum.

#### Phase K — nextest profile + diagnostics (~150 LOC)

- [ ] **K1** Tighten `.config/nextest.toml`: lower retry
      count from 4 to 2 for `mount-integration`, mark
      known-flaky tests with `#[ignore = "tracking #ISSUE"]`,
      add `--no-tests warn`, drop the `leak-timeout = 1s
      pass` override (no longer needed once tests own their
      process tree).
- [ ] **K2** Optional `scripts/test-report.sh` for CI artifact
      upload — categorized markdown report from nextest's
      `--message-format=libtest-json` output.

#### Phase L — Documentation (~200 LOC)

- [ ] **L1** `docs/TESTING.md` updates: "Test fixtures: when
      to use which", "Diagnosing flaky tests" walkthrough,
      schema-hash mechanism explanation.
- [ ] **L2** CLAUDE.md test author checklist (which fixture,
      how to add diagnostic context, when to use the
      test-implementor agent).

#### Explicitly DROPPED from the previous draft

These items from the earlier Phases E–K planning batch are
**not** carried forward because they patch the wrong layer of
the architecture:

- ❌ `scripts/test-mount-clean.sh` — fixtures self-clean
- ❌ `scripts/test-mount-preflight.sh` — fixtures self-clean
- ❌ Build-hash validation in singleton meta files — replaced
  by structurally-impossible-to-mismatch schema-hash-in-path
- ❌ `MountSingletonScope::Owned` opt-in — no singletons exist
  to scope
- ❌ PID-locked sentinel files — no singletons to lock
- ❌ Cross-version singleton compatibility test — wire-format
  mismatch is structurally impossible after Phase F
- ❌ FP domain bulk reset script — handled by reaper self-heal
- ❌ `MountTempDir` panic-hook RAII helper — `command-group`
  handles process cleanup; `tempfile::TempDir` handles file
  cleanup; both run on unwind
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
