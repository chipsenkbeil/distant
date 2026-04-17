# File Mount Branch Decomposition — Design Spec

**Date:** 2026-04-16
**Status:** Approved
**Branch:** `feature/file-mount` (60+ commits ahead of master, 130 files, +24.5k lines)

## Context

The `feature/file-mount` branch adds `distant-mount` crate with 4 mount
backends (FUSE, NFS, macOS FileProvider, Windows Cloud Files), manager-owned
mount lifecycle, network resilience/reconnection, health monitoring, and a
comprehensive rstest-based test suite. The branch compiles clean with zero
stubs and zero merge conflicts with master.

The previous AI development process spiraled into 197 commits due to
whack-a-mole regressions, scope creep, and slow cross-platform verification.
This spec decomposes the branch into reviewable phases that each independently
compile and test, with the test framework introduced early so each backend
adds rstest cases as it lands.

**Chosen approach:** "Foundation First" — decompose into phased PRs, each
with clear completion criteria.

---

## Baseline Stabilization (Pre-Requisite)

- Investigate TD-3 (ProcDone/ProcStdout race) before coding — check git
  history for prior attempts, understand cross-platform differences
- If protocol-level fix is hard, make test assertions more resilient instead
- Time-box to one focused session; don't let it spiral
- Add CI flakiness visibility (report which tests needed retries) regardless
- 7/13 recent "green" CI runs had flaky tests masked by retries — all
  process-spawn/stdout-capture related

---

## Phase Decomposition

### Overview

8 phases (with Phase 3 sub-split into 3a/3b). The test framework enters at
Phase 4 as a skeleton with zero rstest cases. Each subsequent backend phase
adds its `#[case::...]` annotations, so tests grow incrementally.

```
Phase 1 (protocol) → Phase 2 (net resilience) → Phase 3a (events) → Phase 3b (mount lifecycle)
                                                                          ↓
                                              Phase 4 (mount crate + CLI + test skeleton)
                                                                          ↓
                                              Phase 5 (NFS + first rstest cases)
                                                    ↓               ↓              ↓
                                              Phase 6 (FUSE)  Phase 7 (FP)  Phase 8 (WCF)
```

Phases 6, 7, and 8 are independent of each other once Phase 5 lands.

---

### Phase 1: Protocol Consolidation (~1,200 lines)

Collapse `FileRead`/`FileReadText`/`FileWrite`/`FileWriteText`/
`FileAppend`/`FileAppendText` into `FileRead` + `FileWrite` with
`ReadFileOptions`/`WriteFileOptions`. Plus `Msg<T>` custom Deserialize
(wire-protocol error visibility) and `hex_preview` utility.

**Files:**
- `distant-core/src/protocol/request.rs` — variant consolidation
- `distant-core/src/protocol/common/file_options.rs` — new option types
- `distant-core/src/protocol/msg.rs` — custom Deserialize for Msg<T>
- `distant-core/src/api.rs`, `distant-core/src/client/ext.rs` — updated signatures
- `distant-core/src/net/common/utils.rs` — hex_preview
- `distant-host/src/api.rs`, `distant-ssh/src/api.rs`, `distant-docker/src/api.rs`
- `distant-core/tests/api_tests.rs`

**Done when:** `cargo clippy --all-features --workspace` clean, all existing
tests pass, no wire format regressions.

---

### Phase 2: Network Resilience Primitives (~1,100 lines)

TCP keepalive (`TcpTransport::set_keepalive`), heartbeat failure escalation
(`max_heartbeat_failures`), `ReconnectStrategy` variants, `ConnectionWatcher`
improvements, `Plugin::reconnect` + `reconnect_strategy` trait extensions
with default impls. Backend implementations for host/ssh/docker.

**Files:**
- `distant-core/src/net/common/transport/tcp.rs` — keepalive
- `distant-core/src/net/server/config.rs` — heartbeat config
- `distant-core/src/net/server/connection.rs`, `ref.rs` — heartbeat escalation
- `distant-core/src/net/client/reconnect.rs` — strategy types, ConnectionWatcher
- `distant-core/src/net/client/channel.rs` — hex_preview on decode errors
- `distant-core/src/net/common/map.rs` — Map utility
- `distant-core/src/plugin/mod.rs` — reconnect + reconnect_strategy
- `distant-host/src/plugin.rs`, `distant-ssh/src/plugin.rs`, `distant-docker/src/plugin.rs`

**Done when:** All existing tests pass. Reconnect trait methods have unit tests.

---

### Phase 3a: Event Infrastructure + Manager Resilience (~1,200 lines)

Generic event/subscribe system. `EventTopic`, `Event` (with
`ConnectionState` only — no mount variants yet), `Subscribe`/`Unsubscribe`/
`Reconnect` request/response variants, event broadcast bus in manager,
manager reconnection orchestration, `ConnectionWatcher` integration.

**Files:**
- `distant-core/src/net/manager/data/event.rs` — EventTopic, Event (ConnectionState only)
- `distant-core/src/net/manager/data/request.rs` — Subscribe, Unsubscribe, Reconnect
- `distant-core/src/net/manager/data/response.rs` — Subscribed, Unsubscribed, Event, ReconnectInitiated
- `distant-core/src/net/manager/server.rs` — event_tx, subscribe/reconnect handlers
- `distant-core/src/net/manager/server/connection.rs` — reconnection orchestration
- `distant-core/src/net/manager/client.rs` — subscribe/reconnect client methods
- `src/cli/commands/client.rs` — subscribe_and_display_events, reconnect command
- `src/options.rs` — --no-reconnect, --heartbeat-interval, --max-heartbeat-failures

**Done when:** Can subscribe to connection state events via CLI. Unit tests
for event serde round-trips.

---

### Phase 3b: Mount Lifecycle in Manager (~1,500 lines)

Mount protocol types, `MountPlugin`/`MountHandle`/`MountProbe` traits,
manager mount/unmount handlers, `ManagedMount` struct, `Event::MountState`
variant, `monitor_mount` health task, `ResourceKind` for unified List
filtering, manager client mount/unmount methods.

**Files:**
- `distant-core/src/protocol/mount.rs` — MountConfig, CacheConfig, MountInfo, MountStatus, ResourceKind
- `distant-core/src/plugin/mount.rs` — MountPlugin, MountHandle, MountProbe
- `distant-core/src/net/manager/data/event.rs` — add Event::MountState variant
- `distant-core/src/net/manager/data/request.rs` — Mount, Unmount variants
- `distant-core/src/net/manager/data/response.rs` — Mounted, Unmounted, Mounts variants
- `distant-core/src/net/manager/server.rs` — ManagedMount, mount/unmount handlers, monitor_mount
- `distant-core/src/net/manager/server/config.rs` — mount_plugins, mount_health_interval
- `distant-core/src/net/manager/client.rs` — mount/unmount client methods

**Done when:** Unit tests for MountConfig/MountInfo serde, MountPlugin mock
round-trip, mount/unmount via manager with ScriptedMountHandle. Clippy clean.

---

### Phase 4: distant-mount Crate + CLI + Test Framework Skeleton (~3,500 lines)

New `distant-mount` workspace member with core abstractions (RemoteFs,
InodeTable, cache, buffer, handle, runtime), `MountBackend` enum, backend
trait — NO backend implementations. CLI `distant mount`/`distant unmount`/
`distant status --show mount`. Test harness: `MountProcess`, singleton mount
infrastructure, rstest_reuse template with zero `#[case]` annotations.

**Files:**
- `distant-mount/` — crate skeleton (Cargo.toml, src/lib.rs, src/core/*, src/backend/mod.rs, src/plugin.rs)
- `Cargo.toml` — workspace member addition
- `src/options.rs` — mount/unmount/status CLI options
- `src/cli/commands/client.rs` — mount/unmount/status handlers
- `src/cli/common/client.rs`, `src/cli/common/manager.rs` — mount client helpers
- `src/constants.rs` — mount-related constants
- `distant-test-harness/Cargo.toml` — mount feature, distant-mount dependency
- `distant-test-harness/src/mount.rs` — MountProcess, wait helpers, singleton mounts
- `distant-test-harness/src/singleton.rs` — singleton server + mount infrastructure
- `tests/cli/mount/mod.rs` — rstest_reuse template (zero cases)
- `tests/cli/mount/*.rs` — test files (compile but produce zero tests)

**Done when:** Clippy clean. `distant mount` CLI exists and returns "no
backend" error. Test template compiles with zero cases. `cargo test` passes.

---

### Phase 5: NFS Backend + First rstest Cases (~600 lines)

NFS backend (in-process NFS server + os_mount), NFS mount plugin. Add
`#[case::host_nfs]`, `#[case::ssh_nfs]`, `#[case::docker_nfs]` to rstest
template. **First phase where mount tests execute.**

**Files:**
- `distant-mount/src/backend/nfs.rs`
- `distant-mount/src/plugin.rs` — NfsMountPlugin
- `tests/cli/mount/mod.rs` — add NFS case annotations
- `src/cli/commands/manager.rs` — register NFS mount plugin

**Done when:** NFS rstest cases pass for host, ssh, and docker. Full suite green.

---

### Phase 6: FUSE Backend + rstest Cases (~500 lines)

FUSE backend (fuser::spawn_mount2), FUSE mount plugin. Add
`#[case::host_fuse]`, `#[case::ssh_fuse]` to rstest template.

**Files:**
- `distant-mount/src/backend/fuse.rs`
- `distant-mount/src/plugin.rs` — FuseMountPlugin
- `tests/cli/mount/mod.rs` — add FUSE case annotations
- `src/cli/commands/manager.rs` — register FUSE mount plugin

**Done when:** FUSE rstest cases pass for host and ssh backends.

---

### Phase 7: macOS FileProvider Backend + rstest Cases (~2,300 lines)

FileProvider backend with provider/enumerator/item hierarchy, `macos_appex`
binary, provisioning profiles, build scripts, FP-specific test helpers.
Add `#[case::host_file_provider]` to rstest template.

**Files:**
- `distant-mount/src/backend/macos_file_provider.rs` + subdirectory
- `distant-mount/src/plugin.rs` — FileProviderMountPlugin
- `src/macos_appex.rs`, `src/main.rs`, `resources/macos/`, `scripts/`
- `distant-test-harness/src/singleton.rs` — FP singleton
- `distant-test-harness/src/mount.rs` — FP-specific helpers
- `tests/cli/mount/mod.rs` — add FP case annotations

**Done when:** FP rstest cases pass on macOS. Manual verification in Finder.

---

### Phase 8: Windows Cloud Files Backend + rstest Cases (~1,700 lines)

CfApi-based Windows sync root implementation. Add
`#[case::host_windows_cloud_files]` to rstest template.

**Files:**
- `distant-mount/src/backend/windows_cloud_files.rs`
- `distant-mount/src/plugin.rs` — CloudFilesMountPlugin
- `tests/cli/mount/mod.rs` — add WCF case annotations
- `src/cli/commands/manager.rs` — register WCF mount plugin

**Done when:** WCF rstest cases pass on Windows VM.

---

## AI Dev Process Guardrails

### Rule 1: Scope Lock Per Phase
Scope frozen to file list in plan. Out-of-scope → `docs/TODO.md` or issue.

### Rule 2: Autonomous Work with Clean History
AI works autonomously. `cargo fmt + clippy + test` before each commit. May
rebase/amend within current phase.

**Hard stops:** public API/trait changes, phase completion gate, 2 failed
fix attempts.

### Rule 3: No Stacking on Red
Tests fail → fix first. 2 failed attempts → escalate. Never commit broken code.

### Rule 4: Maximum Commit Budget
Soft 10 / hard 15 per phase. Phase 4: soft 12 / hard 18.

### Rule 5: Windows VM On-Demand
Phases 1–3b, 5–6: no VM. Phase 4: compilation check at end. Phase 8: VM at start.

### Rule 6: Code Quality Gate
`code-validator` agent on every commit. No stubs. Blocking issues must be fixed.

### Rule 7: Phase Completion Protocol
1. `cargo clippy --all-features --workspace --all-targets` clean
2. `cargo nextest run --all-features --workspace --all-targets` green
3. Confirm only expected files touched
4. User reviews PR on GitHub before merge
5. Next phase on fresh branch from updated master

---

## Test Reliability

### Singleton Decision
Master uses per-test ephemeral contexts. Branch introduced singletons as a
**performance optimization** (not a technical requirement — FP tests worked
per-test, just slowly). Keep singletons: PID liveness checks, `--shutdown
lonely=30`, unique subdirs, file-lock coordination. Fall back to per-test
if problems arise.

### Rules
- **Poll, don't sleep** — `wait_for_path()`, `wait_until_exists()`, etc.
- **Unique subdirs** — each test gets its own remote directory
- **Hold singleton handles** — name `_sm` or `sm`, never `_`
- **No retry-as-crutch** — NFS/FUSE failures are bugs; FP gets longer
  poll timeouts; WCF gets longer nextest timeouts

### Per-Phase Risk
| Phase | Backend | Risk |
|-------|---------|------|
| 5 | NFS | Low — deterministic |
| 6 | FUSE | Low — similar to NFS |
| 7 | FileProvider | High — materialization timing, appex lifecycle |
| 8 | Cloud Files | High — slow VM |
