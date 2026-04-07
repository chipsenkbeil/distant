# Mount Backends — Production Fixes & Full Test Coverage PRD

## Status (2026-04-07)

**228/228 mount tests passing + 2291 distant-core lib tests passing.**
The Network Resilience + Mount Health work is **complete through Phase
5** (commits `eb0747b` → `9e8e5ea`). PR #288 has been incorporated and
refactored per the review comments to use a generic
[`Subscribe`/`Event`](#wire-types) protocol; mount health monitoring
sits on top of the same event bus.

Highlights:
- Generic `Subscribe { topics: Vec<EventTopic> }` /
  `Subscribed` / `Event { event: Event }` /
  `ReconnectInitiated` protocol covers connection state and mount
  state changes through one canonical bus.
- Per-mount monitor task transitions `MountStatus` (now a typed
  enum) on backend liveness probes and connection state events.
- `distant kill <id>` now tears down mounts on the killed
  connection (HLT-05 regression test locks this in).
- TCP keepalive on every socket; per-plugin reconnect strategies
  (Host/SSH/Docker `ExponentialBackoff`); `--no-reconnect`,
  `--heartbeat-interval`, `--max-heartbeat-failures` CLI flags.
- 26 new unit tests in distant-core for the mount state machine
  and monitor task.

See [PROGRESS.md](PROGRESS.md#active-plan) for the per-step
checklist and
[§ Plan: Network Resilience + Mount Health](#plan-network-resilience--mount-health)
for the full plan that drove this work.

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

### Manual CLI Benchmarks (2026-04-05)

All operations from cold start (no manager running):

| Operation | Time | Notes |
|-----------|------|-------|
| NFS mount | 93ms | Includes NFS server start + OS mount |
| NFS unmount | 186ms | diskutil unmount force + listener shutdown |
| NFS read/write | instant | Files visible immediately |
| FUSE mount | 561ms | Includes fuser::spawn_mount2 |
| FUSE unmount | 49ms | |
| FUSE read/write | instant | Files visible immediately |
| FP mount | 117ms | Domain registered, "visible in Finder" |
| FP read/cat | instant | Working set polling resolves enumeration timing |
| Server start | <1s | |
| Manager start | <1s | |
| Connect | <1s | |

### Original 10-point requirements

- [x] 1. FUSE+SSH EIO — fixed (SFTP error mapping + flush lock + path normalization)
- [x] 2. FileProvider in template — done (singleton via installed app)
- [x] 3. Test shortcuts removed — mount_op_or_skip gone, catch_unwind replaced
- [x] 4. TTL CLI exposure — --read-ttl added
- [x] 5. Readonly — enforced at RemoteFs level for all backends
- [x] 6. TODO.md updated — deferred features documented
- [x] 7. Docker in test matrix — works, offset writes added
- [x] 8. All-green test matrix — 228/228 with zero skips
- [ ] 9. Windows VM script — not started
- [x] 10. Fixed sleeps replaced — polling helpers implemented

### A6 complete

All 37 FP tests pass with zero skips. Fixes: readonly fileSystemFlags +
capabilities, delete/rename handlers, per-mount unmount, remote root
canonicalization, FP-specific test logic for rmdir/unmount/status,
working set polling, wait_for_path helper.

### A7 Phases 1-4 complete: manager-owned mount lifecycle

Architecture: Manager owns mount lifecycle via mount plugins. distant-core
uses generic types (Map/String) — no dependency on distant-mount. Mount
plugins register backends like connection plugins register schemes.

Completed (Phases 1-4):
- [x] MountConfig, MountPlugin, MountHandle traits in distant-core
- [x] 4 plugin implementations (NFS, FUSE, FileProvider, CloudFiles)
- [x] Manager Mount/Unmount handlers with InternalRawChannel
- [x] `distant mount` sends request to manager, exits immediately
- [x] `distant unmount <id>` / `--all` / interactive selection
- [x] `distant status --show mount` replaces `mount-status`
- [x] Async unmount (tokio::process::Command, not blocking)
- [x] macFUSE noappledouble/noapplexattr/nobrowse (Spotlight CPU fix)
- [x] NFS nobrowse/noappledouble/soft/intr (industry-standard options)
- [x] NFS shutdown restructured (unmount before dropping listener)
- [x] Singleton mount infrastructure (16x NFS speedup, 8 parallel threads)
- [x] Docker singleton (persistent container like Host/SSH)
- [x] FP extra metadata injection for manager-owned mounts
- [x] Test harness + status/unmount tests rewritten
- [x] FP enumeration timing — wait_for_path + working set polling

### Status (2026-04-07)

- [x] Health monitoring: periodic per-mount probe + connection
      state propagation
- [x] Connection drop → mount "disconnected" → reconnect → resume
      (via the generic event bus)
- [x] Generic event subscription system (incorporated from PR #288,
      refactored per review)
- [x] HLT-05 regression test for the kill-leak fix
- [ ] HLT-01..04 + EVT-01..02 (sshd kill / connection drop CLI
      tests) — deferred to a follow-up that needs more harness
      orchestration than this round shipped
- [ ] Granular per-backend probes (NFS server task lift, FUSE
      BackgroundSession lift, WCF watcher) — deferred. The
      coarse "mount task ended" signal in Phase 4 is sufficient
      for wholesale-failure detection.
- [ ] Process audit: expect ~5 distant processes (vs 30+ today)
- [ ] Windows testing via ssh windows-vm + rsync + cargo nextest

See [PROGRESS.md](PROGRESS.md) for the detailed checklist and
[§ Plan: Network Resilience + Mount Health](#plan-network-resilience--mount-health)
below for the full step-by-step that drove this work.

Additional completed work not in original requirements:
- Singleton test servers (Host, SSH, Docker, FileProvider)
- Singleton mount infrastructure with file-lock coordination
- Process leak fixes (try_spawn, daemon test rewrite)
- Docker offset write support
- Provisioning profiles checked into repo
- build-macos-app.sh with debug/release profile support
- Remote root canonicalization (symlink resolution at mount time)

## Overview

Complete the mount feature across all 4 backends (NFS, FUSE, Windows Cloud
Files, macOS FileProvider) by fixing production bugs, removing test
workarounds, and achieving a fully green test matrix with zero skips.

This PRD supersedes the original test-only PRD. It covers production code
fixes, test infrastructure improvements, test rewrites, and documentation.

## Requirements (User's 10-Point List)

1. Fix FUSE+SSH EIO bug — writes through FUSE fail when backend is SSH
2. Add FileProvider back to the cross-backend test template
3. Fix all test shortcuts — no `mount_op_or_skip!`, no silent `return`
4. Expose ALL cache TTLs via CLI — `--read-ttl`, FUSE kernel TTL, etc.
5. Investigate readonly native support on WCF and FP; enforce if not native
6. Update `docs/TODO.md` with deferred features (setattr, symlinks, etc.)
7. Docker backend should work in the test matrix
8. Test matrix must be all-green with zero skips
9. Windows Cloud Files testing via separate script (SSH to windows-vm)
10. Replace all fixed sleeps with polling helpers

## Architecture

### Backend x Plugin Test Matrix

|              | Host | SSH  | Docker |
|--------------|------|------|--------|
| NFS          | ✓    | ✓    | ✓      |
| FUSE         | ✓    | ✓    | —      |
| WCF          | ✓*   | —    | —      |
| FileProvider | ✓    | —    | —      |

*WCF runs only on Windows via separate script. Docker+FUSE is not supported.

### rstest_reuse Template (in `tests/cli/mount/mod.rs`)

```rust
#[template]
#[rstest]
#[cfg_attr(feature = "mount-nfs",
    case::host_nfs(Backend::Host, MountBackend::Nfs))]
#[cfg_attr(feature = "mount-nfs",
    case::ssh_nfs(Backend::Ssh, MountBackend::Nfs))]
#[cfg_attr(all(feature = "docker", feature = "mount-nfs"),
    case::docker_nfs(Backend::Docker, MountBackend::Nfs))]
#[cfg_attr(all(feature = "mount-fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")),
    case::host_fuse(Backend::Host, MountBackend::Fuse))]
#[cfg_attr(all(feature = "mount-fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")),
    case::ssh_fuse(Backend::Ssh, MountBackend::Fuse))]
#[cfg_attr(all(feature = "mount-windows-cloud-files", target_os = "windows"),
    case::host_wcf(Backend::Host, MountBackend::WindowsCloudFiles))]
#[cfg_attr(all(feature = "mount-macos-file-provider", target_os = "macos"),
    case::host_fp(Backend::Host, MountBackend::MacosFileProvider))]
fn plugin_x_mount(#[case] backend: Backend, #[case] mount: MountBackend) {}
```

Template is defined in the binary crate (not the harness) so `cfg_attr`
evaluates against the correct feature flags.

### MountProcess Abstraction

`MountProcess` (in `distant-test-harness/src/mount.rs`) handles:
- Spawning the mount process with correct binary (regular or .app bundle)
- Waiting for "Mounted" confirmation on stdout
- Canonicalizing mount paths (macOS `/var` → `/private/var`)
- Backend-specific mount point detection (FP: `~/Library/CloudStorage/`)
- Cleanup on drop: umount -f, diskutil unmount, kill, wait_for_unmount
- FileProvider: builds test .app bundle, registers/removes domain

### Test Pattern

```rust
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn test_name(#[case] backend: Backend, #[case] mount: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    // Seed data via CLI
    let dir = ctx.unique_dir("mount-test-label");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "file.txt"), "content");

    // Mount
    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &["--remote-root", &dir]);

    // Exercise via local filesystem
    let content = std::fs::read_to_string(mp.mount_point().join("file.txt")).unwrap();
    assert_eq!(content, "content");

    // Verify via CLI (not local fs)
    assert!(ctx.cli_exists(&ctx.child_path(&dir, "file.txt")));
}
```

### Sync Verification

Replace `wait_for_sync()` (fixed 2s sleep) with polling helpers:

```rust
/// Poll until a remote file exists, or timeout after 10s.
fn wait_until_exists(ctx: &BackendCtx, path: &str) { ... }

/// Poll until a remote file has expected content, or timeout after 10s.
fn wait_until_content(ctx: &BackendCtx, path: &str, expected: &str) { ... }

/// Poll until a remote path no longer exists, or timeout after 10s.
fn wait_until_gone(ctx: &BackendCtx, path: &str) { ... }
```

### Test File Organization

```
tests/cli/mount/
  mod.rs              — template, mount_op_or_skip macro (to be removed), module list
  browse.rs           — MNT-01..03 (directory listing)
  file_read.rs        — FRD-01..03 (file read)
  subdirectory.rs     — SDT-01..02 (nested directory traversal)
  file_create.rs      — FCR-01..02 (file creation)
  file_delete.rs      — FDL-01..02 (file deletion)
  file_rename.rs      — FRN-01..02 (rename, cross-dir move)
  file_modify.rs      — FMD-01..02 (append, overwrite)
  directory_ops.rs    — DOP-01..03 (mkdir, rmdir, list empty)
  readonly.rs         — RDO-01..03 (readonly enforcement)
  remote_root.rs      — RRT-01..02 (custom root, nonexistent root)
  multi_mount.rs      — MML-01..03 (concurrent mounts, same root)
  status.rs           — MST-01..03 (mount status reporting)
  unmount.rs          — UMT-01..03 (unmount by name/path/all)
  edge_cases.rs       — EDG-01..05 (auto-create, file path, spaces, rapid, stale)
  daemon.rs           — DMN-01 (background mount)
  backend/
    mod.rs
    nfs.rs                 — NFS-specific tests
    fuse.rs                — FUSE-specific tests
    macos_file_provider.rs — FP bundle validation + FP-specific tests
    windows_cloud_files.rs — WCF stub (compile-gated, tested on Windows VM)
```

## Phases

### Phase A: Production Code Fixes

| ID  | Task | Description |
|-----|------|-------------|
| A1  | Fix FUSE+SSH EIO | Investigate and fix write path through FUSE when server is SSH |
| A2  | Readonly enforcement | Native or Rust-level readonly for WCF and FP |
| A3  | TTL CLI exposure | `--read-ttl`, `--fuse-entry-ttl`, `--mount-option KEY=VALUE` |
| A4  | FileProvider in template | FP-aware MountProcess spawn + domain cleanup |
| A5  | TODO updates | Deferred features in `docs/TODO.md` |

### Phase B: Test Infrastructure

| ID  | Task | Description |
|-----|------|-------------|
| B1  | Polling helpers | Replace `wait_for_sync()` with `wait_until_exists/content/gone` |
| B2  | Remove skip macro | After A1, remove `mount_op_or_skip!` |
| B3  | Fix test hacks | FRN-02 cross-dir, MML-03 same-root, RRT-02, MST-03 |
| B4  | FP test fixture | MountProcess builds/spawns .app bundle for FP |
| B5  | Windows VM script | `scripts/test-windows-mount.sh` for remote testing |

### Phase C: Test Quality

| ID  | Task | Description |
|-----|------|-------------|
| C1  | Cross-backend parity | All tests work for all backends (no backend exceptions) |
| C2  | Missing coverage | Large files, Docker+NFS combos, cache TTL |
| C3  | Validation | Run code-validator + test-validator on all code |

### Phase D: Documentation

| ID  | Task | Description |
|-----|------|-------------|
| D1  | MANUAL_TESTING.md | Update with new test results |
| D2  | PRD + progress | Final update of these docs |
| D3  | TODO.md | Deferred features documented |

## Non-Goals

- Stress testing / performance benchmarking
- setattr implementation (pending distant protocol changes)
- Symlink / hard link support (deferred)
- macOS FileProvider App Store signing (test uses ad-hoc)
- Windows CI integration (separate script only)

## Verification

```bash
# All mount tests pass with zero skips on macOS
cargo nextest run --all-features -p distant -E 'test(mount::)'

# Windows tests via SSH (separate)
scripts/test-windows-mount.sh

# Clippy clean
cargo clippy --all-features --workspace --all-targets
```

## Dependencies Between Phases

```
A1 (FUSE+SSH fix) ──→ B2 (remove skip macro) ──→ C1 (cross-backend parity)
A2 (readonly)     ──→ C1
A3 (TTL CLI)      ──→ C2 (TTL tests)
A4 (FP template)  ──→ B4 (FP fixture) ──→ C1
```

---

## Plan: Network Resilience + Mount Health

> **Active plan as of 2026-04-06.** This section is the canonical
> step-by-step for the next chunk of work. It is an embedded copy of
> the planning artefact so that compaction or window-rollover does
> not lose detail. Cross-referenced from
> [PROGRESS.md](PROGRESS.md#active-plan).

### Plan Context

The mount feature suite is now **228/228 green** (commits `df782cd`,
`5b5dcb9`, `86d794d` closed the FP enumeration timing gap).

The next sub-phase in PROGRESS.md is **A7 Phase 5: health monitoring
+ connection resilience**. Originally that meant building
mount-specific status transitions on top of `MountInfo.status` (a
hardcoded `String`). Closer review surfaces a much better starting
point: **PR #288 (`feature/network-resilience`,
chipsenkbeil/distant#288)** already builds the entire connection
health + reconnection stack — TCP keepalive, server-side heartbeat
escalation, `Plugin::reconnect()` / `reconnect_strategy()`,
per-backend health monitors, manager-side `ConnectionWatcher`
plumbing, manager reconnection orchestration, protocol extensions,
CLI flags. ~3300 lines, 9 commits, no merge yet, plenty of unfinished
feedback.

PR #288 was paused on review feedback that targets the **protocol
shape**:

> "Can we not make this a more generic name vs the really long
> `subscribe_connection_events`? Feels like we should make this more
> generic in case there are other events we'd subscribe to in the
> future. We could just call this `subscribe` and then we receive
> `event` or `events` as responses after a `subscribed` response that
> contain the connection events. We'd just need a generic event enum
> that has a type (connection for now) with a specific payload."
> — chipsenkbeil/distant#288 (comment 2933812110)

> "Once again, we should make this be something like `Subscribed`
> and then `Event` (or `Events`) which has an inner type
> `Event(Event)` where the inner type is something like
> `Event::ConnectionStateChanged { id: ConnectionId, state:
> ConnectionState }`." — comment 2933821911

> "Could have the type be `event` and then have a subtype of
> `connection_state_changed` or something like that. Also, that's
> still too long of a name. What is something more concise we can
> use?" — comment 2933826601

The other review threads (separator comments, useless test-section
comments, `pub(crate)` lifts) just need to be respected when porting
the code.

This plan:

1. Brings PR #288 into the file-mount branch as **Step 0**,
   refactored per the review comments into a **generic
   Subscribe / Event protocol** that any subsystem (connection,
   mount, future tunnel, future server-status, etc.) can publish
   to.
2. Then layers mount health on top of that generic event bus
   (**Phases 1–5**), reusing the connection-monitor wiring rather
   than building parallel infrastructure.
3. Moved and updated the docs (`docs/mount-tests-PRD.md` →
   top-level `PRD.md`, `docs/mount-tests-progress.md` →
   `PROGRESS.md`) so future work has a single canonical reference,
   and corrected the stale "9 FP failures" status.

Existing reality vs what PR #288 adds vs what's still needed:

| Today on file-mount | PR #288 introduces | This plan finishes |
|---|---|---|
| `MountInfo.status: String` hardcoded `"active"` | n/a (mount didn't exist on `feature/network-resilience` base) | Typed `MountStatus`, transitions via event bus |
| `ManagerConnection::spawn` discards `clone_connection_watcher()` | Captures watcher, spawns `connection_monitor`, sends to `death_tx` | Same — port verbatim |
| `kill(id)` doesn't touch `self.mounts` (orphan-mount bug) | n/a | Add mount-cleanup loop in `kill` |
| No `Plugin::reconnect()` | Adds default `Unsupported` impl + per-plugin overrides | Same — port verbatim |
| No backend health monitor | SSH (`is_session_closed`), Docker (ping + container state), polled via `ShutdownSender` | Same — port + add NFS/FUSE/FP/WCF mount probes on top |
| No `SubscribeConnectionEvents` | Adds bespoke `SubscribeConnectionEvents` / `ConnectionStateChanged` / `Reconnect` / `ReconnectInitiated` | **Refactor** to generic `Subscribe` / `Subscribed` / `Event(Event)` |
| No CLI subscription helper | `subscribe_and_display_connection_events` | **Generalise** to `subscribe_and_display_events(topics)` |
| No `--no-reconnect`, `--heartbeat-interval`, `--max-heartbeat-failures` | Adds all three | Port verbatim |

### Plan Agent Usage

Per CLAUDE.md plan-mode requirements:

1. **rust-explorer** — already used (two parallel agents) plus a
   third pass over PR #288. Findings baked into this plan.
2. **rust-coder** — implements every step below. Run **once per
   step**, not as one giant batch. The PR brings in 27 files /
   +3296 lines and the mount work cross-cuts most of those, so
   separating concerns is essential.
3. **code-validator** — mandatory after each step that touches
   production code (BLOCKING, max 3 rounds). Steps 0a–0i and 1–4
   each end with a validator pass.
4. **test-implementor** — after each step, ports the upstream
   tests with the same refactors and adds the new mount-specific
   tests in Phase 5.
5. **test-validator** — mandatory after every test-implementor
   run (BLOCKING, max 3 rounds).

No stages skipped. Builtin `Plan` not used past this document.

### Strategy: cherry-pick + refactor in place, commit liberally

PR #288's base (`fb922aa`) is **5 commits behind master** and the
file-mount branch is **173 commits ahead of master**. A `git rebase`
of `pr-288` onto file-mount would explode in
`distant-core/src/net/manager/server.rs` and
`distant-core/src/net/manager/server/connection.rs` (both heavily
modified by mount work). Plan: cherry-pick each PR commit, resolve
by hand, and apply the protocol refactor as part of the cherry-pick
of commit 6 (`feat(core): manager protocol extension for connection
state events`).

`pr-288` is already fetched as a local ref:

```bash
git rev-parse pr-288   # → a12a240a9b7b8d873fd1cd40b39834171ac679a5
git log master..pr-288 --oneline  # → 9 commits
```

**Branch policy:** work directly on the existing `feature/file-mount`
branch. Do **not** create a side branch. Commit liberally — one
commit per sub-step (0a, 0b, 0c, …) so each can be reviewed in
isolation and reverted cleanly if it breaks something downstream.
Format/clippy/nextest must pass before each commit. Keep commit
messages in the existing project style (`feat(core): …`,
`fix(mount): …`).

### Generic Subscribe/Event protocol design

This is the centrepiece refactor. The names are deliberately short
because the comment thread asked for concision.

#### Wire types

In `distant-core/src/net/manager/data/event.rs` (new file):

```rust
//! Generic event types broadcast to subscribed manager clients.

use serde::{Deserialize, Serialize};

use crate::net::client::ConnectionState;
use crate::net::common::ConnectionId;
use crate::protocol::MountStatus;

/// A topic that subscribers can filter on. `All` subscribes to every
/// future event variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTopic {
    All,
    Connection,
    Mount,
}

/// A push notification delivered through the subscription mailbox.
///
/// Each variant carries the minimum payload needed for callers to act
/// on the event without re-querying the manager. Add new variants
/// rather than overloading existing ones.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A managed connection's state changed.
    ConnectionState {
        id: ConnectionId,
        state: ConnectionState,
    },
    /// A managed mount's state changed.
    MountState {
        id: u32,
        state: MountStatus,
    },
}

impl Event {
    /// The topic this event belongs to (used for subscription
    /// filtering).
    pub fn topic(&self) -> EventTopic {
        match self {
            Self::ConnectionState { .. } => EventTopic::Connection,
            Self::MountState { .. } => EventTopic::Mount,
        }
    }
}
```

JSON examples:

```json
{"type":"connection_state","id":42,"state":"reconnecting"}
{"type":"mount_state","id":7,"state":{"state":"failed","reason":"fuse session ended"}}
```

#### Protocol changes

`distant-core/src/net/manager/data/request.rs`:

```rust
pub enum ManagerRequest {
    // ...existing variants...

    /// Subscribe to event notifications. The mailbox stays open
    /// until the client disconnects or unsubscribes.
    Subscribe { topics: Vec<EventTopic> },

    /// Cancel a previous subscription on this channel.
    Unsubscribe,

    /// Manually trigger reconnection of a managed connection.
    Reconnect { id: ConnectionId },
}
```

`distant-core/src/net/manager/data/response.rs`:

```rust
pub enum ManagerResponse {
    // ...existing variants...

    /// Acknowledgement of a `Subscribe` request.
    Subscribed,

    /// Acknowledgement of an `Unsubscribe` request.
    Unsubscribed,

    /// Push notification — only sent on subscribed channels.
    Event(Event),

    /// Acknowledgement that a manual reconnection was started.
    ReconnectInitiated { id: ConnectionId },
}
```

This collapses PR #288's `SubscribeConnectionEvents`,
`SubscribedConnectionEvents`, and `ConnectionStateChanged` into the
generic three-piece `Subscribe` / `Subscribed` / `Event(Event)` and
makes the mount-state event a free addition. Future variants
(`Event::TunnelState`, `Event::ServerStatus`, etc.) plug in without
new request/response types.

#### Server side (`ManagerServer`)

The `event_tx: broadcast::Sender<ManagerResponse>` from PR #288
becomes `event_tx: broadcast::Sender<Event>` (the response wrapping
happens at the per-subscription forwarding task). The
`Subscribe { topics }` handler:

```rust
ManagerRequest::Subscribe { topics } => {
    let mut rx = self.event_tx.subscribe();
    let reply_clone = reply.clone();
    let want_all = topics.contains(&EventTopic::All);
    let topics: HashSet<EventTopic> = topics.into_iter().collect();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if !want_all && !topics.contains(&event.topic()) {
                continue;
            }
            if reply_clone.send(ManagerResponse::Event(event)).is_err() {
                break;
            }
        }
    });
    ManagerResponse::Subscribed
}
```

`notify_state_change(...)` from PR #288 gets renamed to
`publish_event(event_tx, event)` and just calls
`event_tx.send(event)`.

#### Client side (`ManagerClient`)

Replaces PR #288's `subscribe_connection_events()` with:

```rust
pub async fn subscribe(&mut self, topics: Vec<EventTopic>)
    -> io::Result<EventStream>;

pub struct EventStream {
    mailbox: Mailbox<Response<ManagerResponse>>,
}

impl EventStream {
    pub async fn next(&mut self) -> Option<Event> { /* unwrap Event variant */ }
}
```

`subscribe_connection_events(&mut self)` is **removed** (clean
break, 0.21.0). `reconnect(id)` stays the same.

#### CLI helper

`src/cli/common/client.rs::subscribe_and_display_connection_events`
becomes:

```rust
pub async fn subscribe_and_display_events(
    client: &mut ManagerClient,
    topics: Vec<EventTopic>,
    format: Format,
);
```

Shell formatting:

```
[distant] connection 42: reconnecting
[distant] connection 42: connected
[distant] mount 7: failed (fuse session ended)
```

JSON formatting passes the `Event` through serde with a
`{"event":..,"id":..,"state":..}`-style envelope (or just nests the
serialized `Event`).

The long-running CLI commands (Shell, Api, Spawn, Ssh) call
`subscribe_and_display_events(.., vec![EventTopic::Connection,
EventTopic::Mount], format)` instead of the connection-only helper,
so a backgrounded mount drop also surfaces in the shell.

### Step 0 — Land PR #288 (refactored)

Each sub-step is one cherry-pick + manual cleanup. Run `cargo fmt`
/ clippy / nextest after every sub-step.

#### 0a · Move + correct PRD and PROGRESS docs, embed full plan

1. `git mv docs/mount-tests-PRD.md PRD.md`
2. `git mv docs/mount-tests-progress.md PROGRESS.md`
3. Edit `PRD.md` Status section: **228/228 passing, FP suite all
   green, no FP failures remain**. Reword the "FP test failure
   root cause" block as historical/done.
4. Append the entire plan to `PRD.md` as
   `## Plan: Network Resilience + Mount Health`.
5. Edit `PROGRESS.md`:
   - Add `## Active plan` pointer to the PRD section.
   - Add Phase 0 checklist (0a–0j) above the existing A7 Phase 5
     checklist.
   - Mark every previously-`[ ]` box that's actually done by
     228/228.
6. Grep the repo for `docs/mount-tests-PRD.md` and
   `docs/mount-tests-progress.md` and update references.
7. Commit: `docs: move mount PRD/progress to repo root, embed
   network resilience plan, correct 228/228 status`.

#### 0b · TCP keepalive (PR #288 commit 1)

Cherry-pick `61e48c0`. Files:
- `distant-core/Cargo.toml`: socket2 cross-platform.
- `distant-core/src/net/common/transport.rs` + `transport/tcp.rs`:
  add `configure_tcp_keepalive`.
- `distant-core/src/net/common/listener/tcp.rs`: configure on
  accept.

**Address review comment 2933801998** ("Does this one function
need to be pulled up to be available?"): instead of a `pub(crate)
use configure_tcp_keepalive`, add a proper
`TcpTransport::set_keepalive` or `TcpListener::with_keepalive`
method on the public surface and have the listener call that.
Drop the `pub(crate) use` line.

Tests: port the upstream unit tests, drop separator comments and
unnecessary section headers (CLAUDE.md anti-pattern #11).

#### 0c · Heartbeat failure escalation (commit 2)

Cherry-pick `fa40953`. Files:
- `distant-core/src/net/server/config.rs`: add
  `max_heartbeat_failures: u8` (default 3).
- `distant-core/src/net/server/connection.rs`: counter logic.

No protocol exposure yet. Tests ported as above.

#### 0d · `Plugin::reconnect` and `reconnect_strategy` (commit 3)

Cherry-pick `3660e62`. Files:
- `distant-core/src/plugin/mod.rs`: trait extension + tests.
- `distant-host/src/plugin.rs`: ExponentialBackoff (3 retries / 2s
  base / 30s max / 60s timeout).
- `distant-ssh/src/plugin.rs`: ExponentialBackoff (5 retries / 2s
  base / 30s max / 30s timeout).
- `distant-docker/src/plugin.rs`: ExponentialBackoff (10 retries /
  1s base / 60s max / 30s timeout).

Strip separator comments and `// -----` test-section headers per
review comments 2915971580 / 2933755107 / 2933823312.

#### 0e · Backend health monitors (commit 4)

Cherry-pick `993ed8d`. Files:
- `distant-core/src/api.rs`: `ApiServerHandler::from_arc`.
- `distant-core/src/net/server/ref.rs`: `ShutdownSender` + tests.
- `distant-ssh/src/api.rs`: `is_session_closed`.
- `distant-ssh/src/pool.rs`: `is_closed` (delegates to russh
  `Handle::is_closed`).
- `distant-ssh/src/lib.rs`: `ssh_health_monitor` task in
  `into_distant_client` and `into_distant_pair`.
- `distant-docker/src/lib.rs`: `docker_health_monitor` task in
  `into_distant_client` and `into_distant_pair`.

Conflict surface: `distant-docker/src/lib.rs` was significantly
modified for singleton support. Resolve by adding the health
monitor to both into-pair functions without disturbing the
auto-remove cleanup task already there.

#### 0f · `ManagerConnection` connection-watcher plumbing (commit 5)

Cherry-pick `594c3ca`. File:
- `distant-core/src/net/manager/server/connection.rs`: capture
  `client.clone_connection_watcher()` before consuming the client,
  add optional `death_tx`, spawn `connection_monitor`,
  `replace_client`.

Conflict surface: file-mount didn't touch this file structurally,
so the cherry-pick should apply cleanly. Update the existing
unit-test calls to `ManagerConnection::spawn(.., None)` (the new
fourth argument).

#### 0g · Manager protocol — generic events (commit 6, REFACTORED)

This is the load-bearing refactor. Do **not** cherry-pick `5b1c439`
verbatim. Instead, write the new types from scratch as described in
the **Generic Subscribe/Event protocol design** section above.

Files to create/modify:
- `distant-core/src/net/manager/data/event.rs` — **new**, holds
  `EventTopic` and `Event`.
- `distant-core/src/net/manager/data/mod.rs` — re-export.
- `distant-core/src/net/manager/data/request.rs` — add
  `Subscribe { topics }`, `Unsubscribe`, `Reconnect { id }`.
  **Don't** add `SubscribeConnectionEvents`.
- `distant-core/src/net/manager/data/response.rs` — add
  `Subscribed`, `Unsubscribed`, `Event(Event)`,
  `ReconnectInitiated { id }`. **Don't** add
  `SubscribedConnectionEvents` / `ConnectionStateChanged`.
- `distant-core/src/net/client/reconnect.rs` — add `Serialize +
  Deserialize` to `ConnectionState`. Make `initial_sleep_duration`
  and `adjust_sleep` `pub` (used by the orchestration in 0h).

Tests: port the round-trip tests but rename them and replace the
old types. Drop separator comments per review comments
2915971580 / 2933755107 / 2933823312.

#### 0h · Manager reconnection orchestration (commit 7, adapted)

Cherry-pick `aa035a8` and adapt to the new protocol:
- `distant-core/src/net/manager/server.rs`:
  - `connections: Arc<RwLock<HashMap<ConnectionId, ManagerConnection>>>`
  - `death_tx`, `event_tx: broadcast::Sender<Event>` (note `Event`,
    not `ManagerResponse`)
  - `NonInteractiveAuthenticator` (verbatim)
  - `handle_reconnection` (verbatim, but `notify_state_change` →
    `publish_event(event_tx, Event::ConnectionState { id, state })`)
  - Background death-loop in `ManagerServer::new` (verbatim)
  - `ManagerRequest::Subscribe`/`Unsubscribe`/`Reconnect` handlers
    using the design above
  - `ManagerRequest::Reconnect { id }` spawns `handle_reconnection`
    in the background and returns `ReconnectInitiated { id }`

Conflict surface: `server.rs` is the heaviest battleground. The
mount work added the entire `ManagerRequest::Mount` and
`ManagerRequest::Unmount` branches (~150 lines), the `mounts`
field, and `ManagedMount`. PR #288 wraps `connections` in
`Arc<RwLock<...>>` and adds the death loop. Both must coexist.

Resolution recipe:
1. Apply PR #288's struct changes (Arc-wrap connections, add
   death_tx + event_tx).
2. Reapply mount fields (`mounts`) on top.
3. Update `ManagerServer::new` to start with the existing
   `Server::new().handler(Self { ... })` shape and slot the
   death-loop spawn before the `Server::new()` line.
4. Reapply existing handlers (`Mount`, `Unmount`, `List`, etc.)
   inside the new branch list.

#### 0i · CLI integration (commits 8 + 9, generalised)

Cherry-pick `c40c543` and `a12a240`. Adapt:
- `src/cli/common/client.rs`:
  - Replace PR #288's
    `subscribe_and_display_connection_events(client, format)`
    with `subscribe_and_display_events(client, topics, format)`
    accepting a `Vec<EventTopic>`.
  - Render both `Event::ConnectionState` and `Event::MountState`.
- `src/cli/commands/client.rs`:
  - Long-running commands (Shell, Api, Spawn, Ssh) call the new
    helper with `vec![EventTopic::Connection, EventTopic::Mount]`.
  - `distant client reconnect <id>` subcommand uses
    `ManagerClient::reconnect(id)`.
- `src/cli/commands/server.rs`: `--heartbeat-interval`,
  `--max-heartbeat-failures`.
- `src/options.rs`: `--no-reconnect` on Connect/Launch/Ssh.

Heavy conflict surface in `src/cli/commands/client.rs` and
`src/options.rs` from the mount work; resolve by adding the new
flags alongside existing ones.

#### 0j · Validation gate

Run the full pipeline before moving to Phase 1:

```bash
cargo fmt --all
cargo clippy --all-features --workspace --all-targets
cargo nextest run --all-features --workspace
```

Spawn `code-validator` against the full diff for Step 0. Expect
to hit it more than once because of the size of the diff.

### Phase 1 — `MountStatus` enum + protocol-side mount events

Now that the generic event bus exists, mount status can publish
through it.

1. Replace `MountInfo.status: String` with a typed enum at
   `distant-core/src/protocol/mount.rs`:

   ```rust
   #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(tag = "state", rename_all = "snake_case")]
   pub enum MountStatus {
       Active,
       Reconnecting,
       Disconnected,
       Failed { reason: String },
   }
   ```

2. Update `MountInfo.status: MountStatus`. Default constructed as
   `MountStatus::Active`.

3. Update the single writer at
   `distant-core/src/net/manager/server.rs` (~line 604) to use
   `MountStatus::Active`.

4. Update CLI shell rendering at
   `src/cli/commands/client.rs:1473` to match the enum (variant
   name in brackets, `[failed: <reason>]` for `Failed`). The JSON
   path passes through serde unchanged.

5. Add `Event::MountState { id, state }` to the `Event` enum in
   `distant-core/src/net/manager/data/event.rs` (it's already
   declared in 0g but with no producers — Phase 1 adds the first
   producer in Phase 4 below; Phase 1 only ensures the wire shape
   is final).

6. Code-validator pass.

### Phase 2 — `MountHandle::probe` trait extension

1. Add `MountProbe` enum and `probe(&self)` default method to
   `distant-core/src/plugin/mount.rs`:

   ```rust
   #[derive(Clone, Debug)]
   pub enum MountProbe {
       Healthy,
       Degraded(String),
       Failed(String),
   }

   pub trait MountHandle: Send + Sync {
       // ...existing methods...
       fn probe(&self) -> MountProbe { MountProbe::Healthy }
   }
   ```

2. Code-validator pass.

### Phase 3 — `ManagedMount` restructure + per-mount monitor

1. Restructure `ManagedMount` in
   `distant-core/src/net/manager/server.rs`:

   ```rust
   struct ManagedMount {
       info: Arc<RwLock<MountInfo>>,
       handle: Arc<Mutex<Option<Box<dyn MountHandle>>>>,
       manager_channel: ManagerChannel,
       monitor: tokio::task::JoinHandle<()>,
   }
   ```

2. Implement `monitor_mount(info, handle, mut watcher,
   event_tx, interval)`:
   - 5s `tokio::time::interval` (configurable via
     `Config::mount_health_interval`).
   - `select!` between the ticker (calls `handle.probe()`) and
     `watcher.changed()` (uses the connection state).
   - Both branches feed `apply_*` helpers that mutate
     `info.write().await.status` and publish `Event::MountState
     { id, state }` on transitions.
   - `Failed` is terminal — log error, return.

3. Update the `Mount` request branch to:
   - Capture `connection.watch_state()` while the connections
     lock is briefly held.
   - Insert the mount with the new `Arc<RwLock<MountInfo>>`,
     `Arc<Mutex<Option<...>>>`, and freshly spawned `monitor`
     task.
   - Spawn the monitor with a clone of `self.event_tx`.

4. Update the `Unmount` branch to abort the monitor before
   calling `handle.unmount()`.

5. Update the `List { ResourceKind::Mount }` path to clone via
   `info.read().await.clone()`.

6. Add `Config::mount_health_interval: Duration` (default 5s) to
   `distant-core/src/net/manager/server/config.rs`.

7. **Fix the latent kill-leak bug**: in
   `ManagerServer::kill(id)`, after the existing tunnel cleanup,
   add a mount cleanup loop:
   - Snapshot `Vec<u32>` of mount IDs whose
     `info.connection_id == id` (release the read lock).
   - Take the write lock, remove each, abort the monitor, take
     `handle.lock().await.take()` and `unmount().await`, close
     `manager_channel`.

8. Code-validator pass.

### Phase 4 — Backend probe implementations

In `distant-mount/src/plugin.rs`:

- **NFS**: lift `server_task` out of the `async move` closure
  (~line 120) into an `Arc<AtomicBool>` (`server_alive`) flipped
  to `false` by a small watcher task awaiting the join handle.
  Store the flag in `MountHandleWrapper`. `probe()` →
  `Failed("nfs server task exited")` if false. Optional
  follow-up (separate ticket): wire
  `nfsserve::NFSTcp::set_mount_listener` for kernel-side unmount
  detection.

- **FUSE**: lift `BackgroundSession` out of `_session` (~line
  269) into `Arc<Mutex<Option<BackgroundSession>>>` on the
  wrapper. Spawn a `tokio::task::spawn_blocking` that calls
  `session.guard.is_finished()` periodically (or `join`s on a
  separate `Arc<AtomicBool>`). `probe()` → `Failed("fuse session
  ended")` if set.

- **macOS FileProvider**: promote `get_bootstrap_error` and a new
  `runtime_is_ready(&domain_id) -> bool` to public on
  `distant_mount::macos::fp`. `probe()`:
  - `list_file_provider_domains()` doesn't include `domain_id` →
    `Failed("domain not registered")`.
  - `get_bootstrap_error(&domain_id)` returns `Some(err)` →
    `Failed(err)`.
  - `runtime_is_ready(&domain_id) == false` →
    `Degraded("appex not ready")`.
  - Otherwise → `Healthy`.

- **Windows Cloud Files**: replace
  `backend::windows_cloud_files::mount` return type from
  `Box<dyn Any + Send>` with a public typed wrapper. Add an
  `Arc<AtomicBool>` flipped from the watcher thread on exit.
  `probe()` → `Failed("watcher thread exited")` if set.

Code-validator pass.

### Phase 5 — Tests

`test-implementor` writes the following. Place new tests under
`tests/cli/mount/health.rs` (new module, register in
`tests/cli/mount/mod.rs`).

#### Unit tests (`distant-core`)

- `MountStatus` state-machine transitions (positive + negative).
- `Event::topic()` mapping for each variant.
- `Subscribe` filtering: subscriber requesting `[Connection]`
  only receives connection events; subscriber requesting `[All]`
  receives both.
- `monitor_mount` happy path with a scripted test-double
  `MountHandle` that returns canned `MountProbe` values,
  asserting `Arc<RwLock<MountInfo>>` transitions in order **and**
  that an `Event::MountState` was published for each.

#### CLI integration tests

- **HLT-01 healthy steady state**: mount, wait `interval + 1s`,
  `distant status --show mount`, assert `state: active`. Runs
  for every backend in the existing rstest template.
- **HLT-02 connection drop → disconnected**: mount, kill the
  connection (`distant manager kill <id>`), poll `distant status
  --show mount` until status reads `disconnected`, then cleanup.
- **HLT-03 reconnect → active** (SSH only — Host has no remote,
  Docker reconnect is its own beast): mount, kill sshd, restart,
  assert status returns to `active` within 30s. Skip with reason
  for non-SSH backends.
- **HLT-04 backend failure → failed**: backend-specific. For
  FUSE, externally `umount -f` the mount point and assert
  `state: failed`. For NFS, kill the in-process listener task.
  Skip for FP/WCF in the first cut.
- **HLT-05 kill cleans mounts (latent bug regression)**: open
  SSH connection, mount on it, `distant manager kill
  <connection_id>`, verify `distant status --show mount` no
  longer lists the mount.
- **EVT-01 generic subscribe**: spawn a long-running shell
  command, trigger a connection drop, assert the stderr stream
  contains `[distant] connection N: reconnecting` followed by
  `connected` or `disconnected`.
- **EVT-02 mount events arrive on the same subscription**: same
  setup as EVT-01 but with a mount; assert `[distant] mount N:
  failed (...)` shows up after `umount -f`.

Run `test-validator`. Loop until BLOCKING issues are clear.

### Phase 6 — Documentation roll-up

1. Update `PRD.md` Status section with the final scoreboard.
2. Mark the relevant `[ ]` boxes in `PROGRESS.md`.
3. If `docs/CHANGELOG.md` has an unreleased section, add a note
   that `MountInfo.status` is now an enum and that the manager
   protocol gained a generic `Subscribe`/`Event` API.
4. Update `docs/ARCHITECTURE.md` if it documents the manager
   request/response shape.
5. Note the wire-protocol break in the 0.21.0 release notes (no
   compatibility shim — clean break, same as we did for
   `mount-status`).

### Critical files

**Created:**
- `PRD.md` (moved from `docs/mount-tests-PRD.md`, edited)
- `PROGRESS.md` (moved from `docs/mount-tests-progress.md`,
  edited)
- `distant-core/src/net/manager/data/event.rs`
- `tests/cli/mount/health.rs`

**Heavy edits:**
- `distant-core/src/net/manager/server.rs` (Mount/Unmount
  branches, Subscribe/Event handlers, death loop, kill cleanup)
- `distant-core/src/net/manager/server/connection.rs`
  (`connection_watcher`, `replace_client`, `connection_monitor`)
- `distant-core/src/net/manager/data/request.rs` &
  `response.rs` (Subscribe/Event variants)
- `distant-core/src/protocol/mount.rs` (`MountStatus` enum)
- `distant-core/src/plugin/mod.rs` (`reconnect` trait method)
- `distant-core/src/plugin/mount.rs` (`MountProbe` + `probe`)
- `distant-core/src/api.rs` (`from_arc`)
- `distant-core/src/net/server/ref.rs` (`ShutdownSender`)
- `distant-core/src/net/server/config.rs`
  (`max_heartbeat_failures`, `mount_health_interval`)
- `distant-core/src/net/server/connection.rs` (heartbeat counter)
- `distant-core/src/net/common/listener/tcp.rs` &
  `transport/tcp.rs` (TCP keepalive)
- `distant-core/src/net/client/reconnect.rs` (`ConnectionState`
  serde, `pub` accessors)
- `distant-host/src/plugin.rs`,
  `distant-ssh/src/plugin.rs`,
  `distant-docker/src/plugin.rs` (reconnect impls)
- `distant-ssh/src/lib.rs`, `api.rs`, `pool.rs` (SSH health
  monitor)
- `distant-docker/src/lib.rs` (Docker health monitor)
- `distant-mount/src/plugin.rs` (NFS/FUSE/FP/WCF probes — lift
  inner handles)
- `distant-mount/src/backend/macos_file_provider.rs` (public
  `get_bootstrap_error`, `runtime_is_ready`)
- `distant-mount/src/backend/windows_cloud_files.rs` (typed
  `MountGuard`, replace `Box<dyn Any>`)
- `distant-mount/src/lib.rs` (re-exports)
- `src/cli/common/client.rs`
  (`subscribe_and_display_events`)
- `src/cli/commands/client.rs` (long-running commands subscribe;
  `distant client reconnect`)
- `src/cli/commands/server.rs` (`--heartbeat-interval`,
  `--max-heartbeat-failures`)
- `src/options.rs` (`--no-reconnect`)

### Reusable utilities to lean on

- **PR #288 itself** is fetched as `pr-288`. Cherry-pick verbatim
  for everything except the protocol layer (Step 0g).
- `UntypedClient::clone_connection_watcher()` —
  `distant-core/src/net/client.rs:143`.
- `ConnectionWatcher::changed()` /
  `tokio::sync::watch::Receiver::changed()` —
  `distant-core/src/net/client/reconnect.rs`.
- `ConnectionState` serde already in PR #288 (Step 0g promotes
  it fully).
- `ReconnectStrategy::ExponentialBackoff` plus
  `initial_sleep_duration` / `adjust_sleep` accessors (made
  public in 0g).
- `ApiServerHandler::from_arc` (Step 0e).
- `ServerRef::shutdown_sender` / `ShutdownSender` (Step 0e).
- `russh::client::Handle::is_closed` (already used by
  `distant-ssh/src/pool.rs::is_closed` after Step 0e).
- `bollard::Docker::ping` and container-state APIs (already used
  by PR #288's docker_health_monitor).
- `nfsserve::NFSTcp::set_mount_listener` (latent, optional
  follow-up).
- `fuser::BackgroundSession.guard.is_finished()` (stable since
  Rust 1.61).
- `list_file_provider_domains()` —
  `distant-mount/src/backend/macos_file_provider.rs:1413`.
- `get_bootstrap_error` /
  `Runtime.ready: watch::Receiver<bool>` (already exist as
  `pub(crate)`; promoted in Phase 4).
- The tunnel-cleanup-on-kill loop at
  `distant-core/src/net/manager/server.rs:204-216` is the direct
  template for the new mount-cleanup-on-kill loop in Phase 3.

### Open design notes

- **Status vocabulary**: chose `Active / Reconnecting /
  Disconnected / Failed` for `MountStatus`. The connection-side
  enum stays `Connected / Reconnecting / Disconnected` (no
  `Failed` — connections that can't reconnect drop to
  `Disconnected`).
- **`Active` vs `Connected`**: deliberate divergence so the user
  can tell at a glance which subsystem they're looking at and so
  mount-side `Failed` (terminal) is distinguishable from the
  transient connection-side `Disconnected` (recoverable).
- **`Subscribe { topics: Vec<EventTopic> }` vs implicit-all**:
  topics are a `Vec` so a single subscription can receive
  several topic groups without re-subscribing. `EventTopic::All`
  is a shortcut.
- **Event serialization shape**: `#[serde(tag = "type")]` for
  the outer `Event`, plus the existing `#[serde(tag = "state")]`
  on `MountStatus`. JSON consumers can switch on
  `event.type == "mount_state"` and
  `event.state.state == "failed"`.
- **Per-mount vs shared monitor loop**: per-mount, mirroring PR
  #288's per-connection `connection_monitor`. Each monitor dies
  with its mount and uses the local `event_tx` clone — no
  shared state contention.
- **`subscribe_and_display_events` topics for long-running
  commands**: subscribes to both `Connection` and `Mount` so
  the user sees mount failures alongside connection drops. CLI
  may want a per-command override later.
- **WCF `Box<dyn Any>` removal**: still the riskiest single
  change. If it cascades into the binary crate's `cfg`-gated
  handling, fall back to a typed concrete return inside
  `cfg(target_os = "windows")` that downcasts internally rather
  than requiring downstream changes.
- **Test parallelism**: HLT-* tests that drop and reconnect
  connections will be slow. Mark them with `#[ignore]` if they
  exceed the existing mount-suite timing budget — defer to a
  follow-up.

### Plan verification

End-to-end checklist for the implementor:

```bash
# 1. Each Step 0 sub-step ends with these
cargo fmt --all
cargo clippy --all-features --workspace --all-targets
cargo nextest run --all-features --workspace

# 2. After Phase 5, the full mount suite + new HLT/EVT tests
cargo nextest run --all-features -p distant -E 'test(mount::)'
cargo nextest run --all-features -p distant-core -E 'test(mount or event or subscribe)'

# 3. Manual smoke — generic subscription
cargo run --release --all-features -- manager listen &
cargo run --release --all-features -- launch ssh://localhost
cargo run --release --all-features -- shell &
# kill sshd → "[distant] connection N: reconnecting"
# restart sshd → "connected"
# mount + umount -f → "[distant] mount N: failed"

# 4. Manual smoke — kill cleans mounts
cargo run --release --all-features -- mount nfs /tmp/distant-mount
cargo run --release --all-features -- manager kill <connection_id>
cargo run --release --all-features -- status --show mount
# Expect: no mounts listed (regression test for HLT-05)

# 5. Manual smoke — distant client reconnect
cargo run --release --all-features -- client reconnect <connection_id>
```

Acceptance criteria:

1. PR #288's network resilience stack lands on file-mount,
   refactored to use the generic `Subscribe` / `Event` protocol.
2. `MountInfo.status` is a typed enum that serializes through
   serde into a stable JSON shape.
3. Mounts publish `Event::MountState` on transitions through the
   same subscription channel as `Event::ConnectionState`.
4. SSH-backed mounts return to `Active` after a connection drop
   + reconnect cycle within 30s.
5. `distant manager kill <id>` unmounts every mount on that
   connection (regression test for the latent leak bug).
6. All four mount backends implement `MountHandle::probe`.
7. New HLT-* and EVT-* tests pass on macOS.
8. `docs/mount-tests-PRD.md` and `docs/mount-tests-progress.md`
   are moved to `PRD.md` / `PROGRESS.md` at the repo root, and
   both reflect 228/228 + the network-resilience plan.
9. `cargo clippy --all-features --workspace --all-targets` is
   warning-free across the whole workspace.
10. All review comments from PR #288 are addressed (generic
    subscription, no separator/section comments, proper
    `TcpListener::with_keepalive` API rather than `pub(crate)
    use` lift).

---

## Lessons from Phase 0–6 implementation (2026-04-07)

The Network Resilience + Mount Health rollout took 17 commits across
one extended session. The code itself is in good shape, but the test
infrastructure surfaced enough friction to motivate a follow-up
slice ([§ Plan: Test Quality & Stability](#plan-test-quality--stability)
below). This section is the post-mortem inventory: every incident
that cost more than a couple of minutes to diagnose, plus the
underlying root cause.

### Stale singleton state was the #1 friction source

Phase 1 changed `MountInfo.status` from `String` to a `MountStatus`
enum. The wire format went from `"status":"active"` to
`"status":{"state":"active"}`. When I ran the integration suite
afterwards, **every FP test failed silently with "No mounts
found"**, not with a deserialization error. Root cause: the
singleton manager process was started by an OLD binary
(pre-Phase-1) and stayed alive across cargo invocations because
its `--shutdown lonely=N` timer hadn't expired. The NEW client
binary tried to deserialize the OLD wire format and silently
produced an empty list.

I diagnosed this by accident — `pkill -f "Distant.app/Contents/MacOS/distant"`
+ removing the lock files, then re-running, and seeing every test
go green. Without `pkill` knowledge I might have spent hours on
the wrong path (e.g. hunting for a regression in my Phase 1 code).

**Direct fixes**: Phase E1 (cleanup script), E2 (build hash
validation in singleton meta files so the next attach detects the
mismatch and tears the singleton down rather than silently
failing).

### "No mounts found" was uninformative

The status integration test fails with:
```
panicked at tests/cli/mount/status.rs:28:5:
[Host/macos-file-provider] status --show mount should include
backend name 'macos-file-provider', got:
No mounts found
```

That message tells me NOTHING about which manager was queried,
which binary handled the request, what socket was used, or what
the manager actually returned. To diagnose I had to (a) re-run
with `--no-capture` (a flag I had to Google), (b) discover which
PID the test reused vs spawned, (c) `tail` the manager log file
manually.

**Direct fixes**: Phase F1 (verbose failure context macro), F2
(diagnostic dump on the singleton handle), F3 (inline log tail
in panic messages).

### Test harness compilation was fragile under feature subsets

When I tried to run `cargo test --all-features -p distant-host
--lib` to verify a small slice of the workspace, the
`distant-test-harness` failed to compile:

```
error[E0433]: failed to resolve: could not find `mount` in the
crate root
   --> distant-test-harness/src/singleton.rs:422:12
    |
422 |     crate::mount::install_test_app().expect(...);
```

Root cause: `start_file_provider` referenced
`crate::mount::install_test_app()` unconditionally even though
the `mount` module was gated behind `feature = "mount"`. This
was a pre-existing bug; I fixed it as a one-line cfg gate
(commit `2b1a2bf`), but the underlying problem is that the test
harness has multiple cross-cutting feature gates and no CI job
that builds with subset features.

**Direct fixes**: Add a CI matrix that builds the test harness
with the cartesian product of feature flags (covered by the
broader Phase J work).

### Cherry-pick conflict resolution was lossy

PR #288 was based on a branch behind the file-mount work. Each
of the 9 cherry-picks had at least one conflict in `server.rs`
or `lib.rs`. I resolved them by hand. The resolution was correct
each time, but:

- I had to manually re-strip separator comments (anti-pattern
  #11) in three plugins because the cherry-pick re-introduced them
- I had to manually rename `notify_state_change` to
  `publish_connection_state` in two locations across two
  commits (a third one was missed and only caught by `cargo
  check`)
- I had to manually fix the `MountInfo.status` field initializer
  twice — once in the production code (Phase 1 commit `ae850c5`)
  and once in a test fixture that the cherry-pick brought in but
  didn't update

**Direct fixes**: Phase H1 (frozen wire-format fixtures) catches
breaking protocol changes the moment they ship, not the next
time someone runs the integration suite against a stale
singleton.

### Tests didn't catch the orphan-mount latent bug

Until HLT-05 was added in Phase 5, no test covered the case
"connection killed → mounts cleaned up". The bug had probably
been latent since A7 Phase 4 (manager-owned mount lifecycle)
shipped. It was only spotted because the explore agent in
Phase 0 read the `kill(id)` code carefully and noticed the
omission.

**Direct fixes**: Phase H2 ships HLT-01..04 + EVT-01..02 to
cover the rest of the lifecycle scenarios. Phase H5 covers the
per-backend probe failure modes that the coarse Phase 4 probe
can't catch.

### Background tasks vs foreground tasks vs timeouts

Several `cargo nextest run` invocations during this session
either ran in the background and timed out (e.g. one I had to
`TaskStop`), or hung indefinitely waiting on a stale singleton
that was never going to respond. The default `--no-fail-fast
--test-threads=1` invocation I used for diagnostics is slow
(~one test per second of overhead) but easier to read.

**Direct fixes**: Phase J2 (preflight script) avoids the stale
singleton hangs, J1 (tighter retry policy) makes flakes surface
faster, J3 (test result triage) makes async runs easier to skim.

### Build cycle is 10–30s of latency between commits

Each commit needed `cargo fmt + cargo clippy + cargo test`.
On the file-mount branch with my hardware, this was ~25 seconds
of wall clock per cycle. Across 17 commits that's ~7 minutes of
pure linker latency. Not catastrophic but adds up.

**Direct fixes**: Phase I4 documents the `mold`/`lld` linker
setup and adds a `dev-fast` profile.

### Test author boilerplate is too high

The HLT-05 test I wrote in Phase 5 had two CLI subcommand typos
on the first attempt:
- `manager list` (doesn't exist; correct is `status --show
  connection`)
- `client kill` (doesn't exist; correct is just `kill`)

Both took one full test run + grep + edit to fix. A typed
command builder would have caught both at compile time.

**Direct fixes**: Phase I1 (typed `DistantCmd` builder), I2
(reusable fixtures so a test author doesn't have to assemble
connect+mount+verify from scratch each time).

### Flakes are masked by retries

The full mount integration run reported `228 tests run: 228
passed (3 slow, 6 flaky, 1 leaky)`. The 6 flakes silently
retried up to 5 times each and eventually passed. That's
acceptable for landing a green build, but it papers over real
intermittent issues that will get worse as the test suite
grows. There's no tracking issue list for the flaky 6, no
characterization of WHICH backends they hit, and no decision
to either fix them or quarantine them with `#[ignore]`.

**Direct fixes**: Phase J1 (lower retry budget so flakes show
up), Phase H4 (soak tests to characterize the underlying
leaks).

---

## Test Architecture Today (2026-04-07)

> **Diagrams precede the plan** so the failure surface is visible
> before the proposed fix. Sources: research-only inventory of
> `distant-test-harness/src/{singleton,mount,manager}.rs`,
> `tests/cli/mount/**`, `.config/nextest.toml`, plus three deep
> research passes (cleanup patterns, nextest internals, mount test
> inventory). Full reports archived under `~/.claude/plans/`.

### Diagram 1 — How a mount test runs today (singleton path)

```
                           ┌──────────────────────────────────────────┐
                           │  $TMPDIR/distant-test-<hash>-host.lock   │
  ┌────────────────────┐   │  $TMPDIR/distant-test-<hash>-host.meta   │
  │  cargo nextest run │   │  $TMPDIR/distant-test-<hash>-host.sock   │
  │   (test process    │   └──────────────────────────────────────────┘
  │    spawned per     │                       ▲
  │    test by         │                       │ fs4 file lock
  │    nextest)        │                       │ JSON meta with manager_pid
  └────────┬───────────┘                       │
           │                                   │
           │ get_or_start_host()               │
           ▼                                   │
  ┌────────────────────────────────────────────┴─┐
  │  read_live_meta()                            │
  │   ├─ is_pid_alive(manager_pid)?              │
  │   ├─ if dead → cleanup_meta(), respawn       │
  │   └─ if alive → return existing socket path  │
  └─────────┬────────────────────────────────────┘
            │
            │ (if respawn needed)
            ▼
  ┌──────────────────────────────────────────────┐         ┌─────────────────┐
  │  Spawn singleton processes                   │ ─────►  │ distant manager │
  │   ├─ distant manager listen --shutdown       │         │   listen        │
  │   │     lonely=30 ...                        │         │   PID stored in │
  │   ├─ distant server listen --shutdown        │         │   meta JSON     │
  │   │     lonely=60 ...                        │         └────────┬────────┘
  │   ├─ distant connect <credentials>           │                  │
  │   └─ std::mem::forget(mgr); forget(server);  │                  │
  └──────────┬───────────────────────────────────┘                  │
             │                                                       │
             │ then for mount tests:                                 │
             ▼                                                       │
  ┌──────────────────────────────────────────────┐                  │
  │  get_or_start_mount(ctx, mount_backend)      │                  │
  │   ├─ same pattern: lock + meta + spawn       │                  │
  │   └─ MountSingletonHandle holds shared lock  │                  │
  └──────────┬───────────────────────────────────┘                  │
             │                                                       │
             ▼                                                       ▼
  ┌──────────────────────────────────────────────┐         ┌─────────────────┐
  │  Test body                                   │  ────►  │   distant       │
  │   - reads/writes inside unique_subdir of     │  ────►  │   server        │
  │     shared remote_root                       │  ────►  │   PID stored    │
  │   - asserts via std::fs + ctx.cli_*          │         │   in meta JSON  │
  └──────────┬───────────────────────────────────┘         └─────────────────┘
             │
             │ test body returns (or panics)
             ▼
  ┌──────────────────────────────────────────────┐
  │  Drop chain                                  │
  │   ├─ MountSingletonHandle::drop  → no-op     │
  │   │      (only releases shared lock)         │
  │   ├─ BackendCtx::drop → owns_processes=false │
  │   │      → no-op                             │
  │   └─ TempDir::drop → cleans local temp dir   │
  └──────────────────────────────────────────────┘
             │
             ▼
       Test process exits.
       Singleton manager + server keep running.
       Next test reuses them.
```

**Cargo test variant**: same singleton machinery, but all tests in a
binary share the same in-memory `OnceLock`s as well. The first test
in the binary forks the singletons; subsequent tests in the same
binary attach via the in-memory cache before even checking the
file lock. Under nextest, every test is a fresh process, so the
in-memory cache is always cold and the file lock is the only
coordination.

### Diagram 2 — Where it breaks

```
  ╔════════════════════════════════════════════════════════════════╗
  ║  Failure surface inventory                                     ║
  ╠════════════════════════════════════════════════════════════════╣
  ║                                                                ║
  ║  ① Wire format mismatch                                        ║
  ║    Old singleton (binary built BEFORE Phase 1) holds the lock  ║
  ║    file. New test client (binary built AFTER Phase 1) attaches ║
  ║    and sends a new-shape request. Manager rejects or returns   ║
  ║    a different shape than expected.                            ║
  ║      Symptom: silent "No mounts found" — no error, just empty  ║
  ║    Detection time today: hours                                 ║
  ║                                                                ║
  ║  ② SIGKILL of test process                                     ║
  ║    User hits Ctrl+C, nextest timeout, OOM.                     ║
  ║    No destructors run.  std::mem::forget'd children outlive    ║
  ║    parent. Children are NOT in parent's pgid (set_process_group║
  ║    detaches them on purpose), so killpg can't reap them.       ║
  ║      Symptom: orphaned distant manager / server / sshd /       ║
  ║    docker containers / FP CloudStorage entries piling up       ║
  ║                                                                ║
  ║  ③ Singleton pid is dead but meta is stale                     ║
  ║    Process killed externally (e.g. user-initiated kill -9).    ║
  ║    is_pid_alive() catches it ONLY for manager_pid. Server,     ║
  ║    sshd, container PIDs are not probed.                        ║
  ║      Symptom: stale endpoint, ConnectionRefused after 1-2s     ║
  ║                                                                ║
  ║  ④ Concurrent cleanup_all_stale_mounts                         ║
  ║    `status::status_no_mounts_should_say_none` runs            ║
  ║    `cleanup_all_stale_mounts()` which force-unmounts EVERY     ║
  ║    NFS/FUSE mount in the OS mount table. With nextest's        ║
  ║    `mount-integration` group at max-threads=8, this races      ║
  ║    other tests' singleton mounts.                              ║
  ║      Symptom: random EIO / "mount disappeared" failures        ║
  ║                                                                ║
  ║  ⑤ install_test_app mid-suite                                  ║
  ║    `backend/macos_file_provider::install_test_app` swaps       ║
  ║    /Applications/Distant.app while a sibling FP test may       ║
  ║    have the singleton manager running with that binary         ║
  ║    memory-mapped.                                              ║
  ║      Symptom: undefined behavior. Mostly works, occasionally   ║
  ║    catastrophic                                                ║
  ║                                                                ║
  ║  ⑥ Singleton state accumulation                                ║
  ║    `unique_subdir` files leak inside the shared remote_root    ║
  ║    forever. `read_should_handle_large_file` leaks 100 KB per   ║
  ║    run. `~/Library/CloudStorage/` accumulates 60+ stale FP     ║
  ║    domain dirs over time.                                      ║
  ║      Symptom: slow test startup, unbounded disk growth         ║
  ║                                                                ║
  ║  ⑦ HostManagerCtx::start without lonely shutdown               ║
  ║    Owned isolated managers have no `--shutdown lonely=N`       ║
  ║    flag. If Drop is skipped (panic-abort, SIGKILL), they       ║
  ║    leak forever.                                               ║
  ║                                                                ║
  ║  ⑧ Two duplicate plugin_x_mount templates                      ║
  ║    distant-test-harness/src/mount.rs:662 (unused) and          ║
  ║    tests/cli/mount/mod.rs:20 (used). Will drift apart.         ║
  ║                                                                ║
  ║  ⑨ Inconsistent test ownership model                           ║
  ║    Five different "I need a mount to test" patterns across     ║
  ║    16 mount test files: get_or_start_mount, MountProcess::     ║
  ║    spawn, MountProcess::try_spawn, HostManagerCtx::start,      ║
  ║    raw CLI invocation. Test author has to pick the right one   ║
  ║    for each new test.                                          ║
  ║                                                                ║
  ║  ⑩ Five different fixtures, all failing the same way on crash ║
  ║    Manager/server/sshd/docker/FP_appex are all spawned with    ║
  ║    std::mem::forget; none of them have a path-of-no-cleanup    ║
  ║    that survives SIGKILL                                       ║
  ║                                                                ║
  ╚════════════════════════════════════════════════════════════════╝
```

### Diagram 3 — Process tree on a typical test run (today)

```
  cargo nextest run
       │
       └─ test_runner_for_each_test (50 children, one per test)
              │
              ├─ test_proc_1 ──── distant CLI ──┐
              │                                  │ socket path
              ├─ test_proc_2 ──── distant CLI ──┤  from .meta
              │                                  │
              └─ test_proc_50 ─── distant CLI ──┘
                                                 │
                                                 ▼
                                  ┌──────────────────────────────┐
                                  │  ORPHANED singleton tree     │
                                  │                              │
                                  │   distant manager (PG = own) │
                                  │     PID 12345                │
                                  │                              │
                                  │   distant server  (PG = own) │
                                  │     PID 12347                │
                                  │                              │
                                  │   sshd (test fixture)        │
                                  │     PID 12350                │
                                  │     ↳ /tmp/distant-test-     │
                                  │       <hash>-sshd-XXXX/      │
                                  │       (TempDir LEAKED)       │
                                  │                              │
                                  │   docker container test-XXXX │
                                  │     (sleep infinity)         │
                                  │                              │
                                  │   /Applications/Distant.app/ │
                                  │     Contents/MacOS/distant   │
                                  │     (FP singleton manager,   │
                                  │      different binary path!) │
                                  │                              │
                                  │   DistantFileProvider.appex  │
                                  │     (loaded by macOS)        │
                                  └──────────────────────────────┘
                                              │
                                              │ pgid != cargo's pgid
                                              │ no PR_SET_PDEATHSIG
                                              │ no Ryuk
                                              ▼
                                  Survives SIGKILL of cargo.
                                  Survives Ctrl+C.
                                  Survives panic-abort.
                                  Lives forever unless killed manually.
```

### Diagram 4 — Test inventory by mount-source pattern

```
                                       16 test files
                                       39 test functions
                              ┌────────┴────────┐
                              │  How they get a │
                              │     mount       │
                              └────────┬────────┘
                                       │
        ┌──────────────────────────────┼──────────────────────────────┐
        ▼                              ▼                              ▼
   ┌─────────────┐             ┌──────────────┐             ┌─────────────┐
   │  Singleton  │             │ MountProcess │             │  Isolated   │
   │  share-all  │             │   per-test   │             │   manager   │
   │             │             │              │             │  (already!) │
   │  14 tests   │             │  12 tests    │             │   2 tests   │
   └─────────────┘             └──────────────┘             └─────────────┘
   browse #1                   browse #2-3                  health::kill_*
   directory_ops 1-3           edge_cases 1-2,5             unmount::all_*
   edge_cases 3-4              multi_mount 1-3
   file_create 1-2             readonly 1-3
   file_delete 1-2             remote_root 1-2              + 1 test that
   file_modify 1-2             unmount::by_id                bypasses everything
   file_read 1-3                                             (status::no_mounts)
   file_rename 1-2
   subdirectory 1-2
   status #1-2
   backend/nfs
   backend/fuse

   Each leaks files into          Each owns its own mount     Each owns its own
   shared remote_root forever     atop the SHARED manager     manager+server.
   via unique_subdir                                          ALREADY isolated.

   ┌─────────────────────────────────────────────────────────────────┐
   │  KEY INSIGHT: only 2 of 39 tests genuinely need a singleton.    │
   │  The other 37 either don't share state at all (12) or only      │
   │  share for performance, not correctness (14 + 11 read-only).    │
   │  The "singleton everywhere" model exists ONLY because spawning  │
   │  a fresh manager+server per test was slow.                      │
   └─────────────────────────────────────────────────────────────────┘
```

### Diagram 5 — The proposed architecture

```
                  ┌──────────────────────────────────────────────┐
                  │  cargo nextest run / cargo test              │
                  │   (works identically — no separate paths)    │
                  └──────────────────┬───────────────────────────┘
                                     │
              ┌──────────────────────┼─────────────────────────┐
              │                      │                         │
              ▼                      ▼                         ▼
   ┌─────────────────┐    ┌─────────────────┐       ┌──────────────────┐
   │  test_proc_1    │    │  test_proc_2    │       │  test_proc_N     │
   │   (host nfs)    │    │   (ssh fuse)    │       │   (host fp)      │
   └────────┬────────┘    └────────┬────────┘       └────────┬─────────┘
            │                      │                          │
            │ Ephemeral            │ Ephemeral               │ Lease
            │ via                  │ via                    │ from
            │ command-group        │ command-group         │ reaper
            ▼                      ▼                          ▼
   ┌─────────────────┐    ┌─────────────────┐       ┌──────────────────┐
   │ FreshManager    │    │ FreshManager    │       │ FpFixtureLease   │
   │  (own pgid,     │    │  (own pgid,     │       │  (UDS conn to    │
   │   own port,     │    │   own sshd,     │       │   distant-reaper)│
   │   own files)    │    │   own port)     │       │                  │
   │                 │    │                 │       │  Drops on test   │
   │  Drop:          │    │  Drop:          │       │  exit; reaper    │
   │   killpg(pgid,  │    │   killpg(pgid,  │       │  sees connection │
   │     SIGTERM)    │    │     SIGTERM)    │       │  close → linger  │
   │   wait + reap   │    │   wait + reap   │       │  → cleanup       │
   └─────────────────┘    └─────────────────┘       └────────┬─────────┘
                                                              │
                                                              ▼
                                                ┌─────────────────────────┐
                                                │ distant-reaper sidecar  │
                                                │ (~300 LOC binary)       │
                                                │                         │
                                                │ Listens on              │
                                                │ /tmp/distant-reaper-    │
                                                │   <schema_hash>.sock    │
                                                │                         │
                                                │ Owns FP singleton tree: │
                                                │  ├─ FP manager          │
                                                │  ├─ FP server           │
                                                │  └─ FP appex            │
                                                │                         │
                                                │ Self-heals stale state  │
                                                │ on startup. Schema-hash │
                                                │ in path means stale     │
                                                │ reapers from old binary │
                                                │ are on a different path │
                                                │ entirely.               │
                                                └─────────────────────────┘
```

**Why this design satisfies the constraints**:

| Constraint | How it's satisfied |
|---|---|
| Works for both `cargo test` and `cargo nextest` with one code path | The fixture types (`MountedHost`, `FpFixtureLease`) are constructed identically in both. cargo test still benefits from in-process `OnceLock` caching of the reaper handle, but the cleanup path is the same. |
| No external scripts | Reaper is a Rust binary in the workspace, spawned from inside Rust code on first test access. nextest setup scripts are not used. |
| SIGKILL of test process is handled | `command-group::group_spawn` puts each fresh manager in its own pgid; on Linux, `pre_exec` calls `prctl(PR_SET_PDEATHSIG, SIGTERM)` so the manager dies with the parent. On macOS, a kqueue thread inside the manager watches `getppid()` and exits on parent death. The FP reaper is NOT in the test process's tree at all — it self-cleans via lease-socket disconnect detection. |
| Wire format mismatch detected immediately | Schema hash is baked into the reaper socket path. New binary → new path → fresh reaper. Old reaper times out on its lonely shutdown after its last lease disconnects. No silent failures. |
| Eliminates `pkill` / `rm -f` rituals | Per-test fixtures clean up on drop. The reaper self-heals stale state on startup. No external cleanup needed. |
| Doesn't compromise test coverage | Every existing test still runs; the test BODIES don't change. Only the *fixture* layer is replaced. |

### Diagram 6 — What changes per test category

```
                ┌─────────────────────────────────────────┐
                │  37 / 39 tests: ephemeral fixture       │
                ├─────────────────────────────────────────┤
                │                                         │
                │   #[fixture] fn host() -> MountedHost   │
                │                                         │
                │   #[rstest]                             │
                │   fn browse_should_list(host: MountedHost) {       │
                │       let mp = host.mount_nfs("/tmp").unwrap();    │
                │       // ... test body unchanged ...               │
                │   }   // mp drops, mount unmounted                 │
                │       // host drops, manager+server killpg'd       │
                │                                         │
                │  No singleton. No shared state. No      │
                │  unique_subdir. No mem::forget. No      │
                │  cleanup_all_stale_mounts. Each test    │
                │  is hermetic.                           │
                │                                         │
                └─────────────────────────────────────────┘

                ┌─────────────────────────────────────────┐
                │  2 / 39 tests: FP appex (lease)         │
                ├─────────────────────────────────────────┤
                │                                         │
                │   #[fixture] fn fp() -> FpFixtureLease  │
                │                                         │
                │   #[rstest]                             │
                │   fn read_through_fp(fp: FpFixtureLease) {         │
                │       let mp = fp.mount("/tmp").unwrap();          │
                │       // ... test body unchanged ...               │
                │   }   // fp drops, lease socket closes             │
                │       // reaper sees disconnect, lingers, cleans   │
                │                                         │
                │  Single sidecar. Schema-hashed path.    │
                │  Auto-reaps on SIGKILL via lease.       │
                │                                         │
                └─────────────────────────────────────────┘
```

---

## Plan: Test Quality & Stability (revised 2026-04-07)

> **Active plan as of 2026-04-07.** This is the next slice after
> Network Resilience + Mount Health. **This revision REPLACES the
> earlier draft of Phases E–K** with a smaller, more focused
> design driven by ~30 minutes of dedicated research into nextest
> internals, ctor/dtor crates, process supervision, and the actual
> test infrastructure code. Each phase below maps to a specific
> incident in
> [§ Lessons from Phase 0–6 implementation](#lessons-from-phase-06-implementation-2026-04-07)
> AND a concrete weakness in the diagrams above. Cross-referenced
> from
> [PROGRESS.md § Phases E–K](PROGRESS.md#phases-ek--test-quality--stability-next-slice).

### Plan goals (revised)

The earlier draft of this plan tried to *patch* the singleton-for-
everything model with cleanup scripts, build-hash checks, and
diagnostic helpers. After 30+ minutes of dedicated research into
nextest internals, the ctor/dtor model, process supervision, and
the actual harness code, the conclusion is:

> **The singleton model itself is the bug.** Replace it with
> per-test ephemeral fixtures for the 80% case, and a tiny
> Ryuk-style sidecar reaper for the one true singleton (the FP
> appex). This is fewer lines of code, fewer moving parts, and
> eliminates entire categories of failure mode at the source.

The new goals are:

1. **One ownership model per test category.** 37 of 39 mount
   tests get ephemeral per-test fixtures (no shared state, no
   `unique_subdir`, no `cleanup_all_stale_mounts` landmines).
   2 tests that genuinely need a singleton (FP appex) use a
   typed `FpFixtureLease`. No more "five different patterns
   for getting a mount."
2. **SIGKILL is handled by the OS, not by hope.** Per-test
   fixtures use `command-group` (Linux/macOS pgid + Windows job
   objects) so killpg-on-Drop reaps every grandchild. The FP
   reaper uses a Ryuk-style connection lease so SIGKILL of the
   test process is observable from outside the dying tree.
3. **Wire-format mismatches become impossible.** Each ephemeral
   fixture uses the binary the test was built from — no stale
   binaries, no cached singletons. The FP reaper bakes the
   wire-format schema hash into its socket path, so a binary
   built from a different schema goes to a different reaper,
   period.
4. **Cleanup is invisible to the test author.** No scripts to
   run, no `pkill` rituals, no preflight steps. `cargo test`,
   `cargo nextest run`, even `cargo run -p distant ...` work
   identically without manual hygiene.
5. **Coverage NEVER decreases.** Every test that exists today
   keeps existing. Every assertion stays. The fixture LAYER
   changes; test BODIES are mostly untouched. The two tests
   that exercise `MountProcess::Drop` semantics (`browse::drop_should_unmount`,
   `edge_cases::drop_should_leave_no_stale_mounts`) keep
   working because the new fixtures use the same Drop pattern.

### What the OLD plan got right and what was wasted

| Old phase | New plan | Why |
|---|---|---|
| **E1** cleanup script | **DROPPED** | Not needed once fixtures self-clean. No external scripts to maintain. |
| **E2** build-hash validation | **MERGED into Phase F** as schema-hash-in-reaper-socket-path. Smaller, more focused. |
| **E3** stale FP domain cleanup | **MERGED into Phase G** as FP reaper startup. Self-heals on launch. |
| **F1** assert_mount_status! macro | **KEPT as Phase H1** | Useful regardless of fixture model. |
| **F2** singleton diagnostic dump | **REPLACED with per-fixture diagnostics** | New fixtures have their own diagnostic dump because they own the manager. |
| **F3** panic hook log dump | **KEPT as Phase H2** | Useful regardless. |
| **G1** Owned-singleton scope | **OBVIATED** | Default model IS owned. No scope choice needed. |
| **G2** PID-locked sentinels | **OBVIATED** | No singleton to lock. |
| **G3** RAII tempdirs | **OBVIATED** | command-group + tempfile already RAII. |
| **H1** wire format fixtures | **KEPT as Phase J1** | Still want forward-compat checking. |
| **H2** HLT-01..04 + EVT-01..02 | **KEPT as Phase J2** | Still the deferred coverage gap. |
| **H3** cross-version compat test | **OBVIATED** | Schema-hash-in-path makes mismatch impossible. |
| **H4** soak / leak tests | **KEPT as Phase J3** | Still useful. |
| **H5** per-backend probe tests | **KEPT as Phase J4** | Still useful. |
| **H6** proptest round-trips | **KEPT as Phase J5** | Still useful. |
| **I1** DistantCmd builder | **KEPT as Phase I1** | Independent win. |
| **I2** fixture set | **PROMOTED** to Phase G — this IS the refactor now. |
| **I3** mock MountHandle | **KEPT as Phase I3** | Independent. |
| **I4** dev-fast profile | **KEPT as Phase I4** | Independent. |
| **J1** nextest profile tweaks | **KEPT as Phase K1** | |
| **J2** preflight script | **DROPPED** | Not needed. |
| **J3** test result triage | **KEPT as Phase K2** | |
| **K1** TESTING.md updates | **KEPT as Phase L1** | |
| **K2** test author checklist | **KEPT as Phase L2** | |

**Result**: the new plan is ~40% smaller in surface area, focuses
the high-value work on the architectural fix, and drops the
"patch the singleton model" sub-phases that don't actually
eliminate the failure modes.

### Plan agent usage

Same pipeline as Network Resilience + Mount Health:

1. **rust-explorer** — already used for the inventory + research
   above. Use again to verify each phase before committing.
2. **rust-coder** — implements each phase. **One commit per
   sub-phase** to keep blast radius small.
3. **code-validator** — mandatory after each step that touches
   production code or harness code (BLOCKING).
4. **test-implementor** — for the HLT/EVT tests in Phase J2 and
   any new fixtures. **Use the agent instead of writing tests
   directly** — the HLT-05 test had two CLI typos that test-
   validator would have caught.
5. **test-validator** — mandatory after every test-implementor
   run (BLOCKING).

### Phases at a glance (revised)

| Phase | Theme | Key deliverable | LOC | Risk |
|---|---|---|---|---|
| **E** | Wire-format hardening | `#[serde(other)]` fallback variants on every wire enum + schema hash function | ~50 | Trivial |
| **F** | Schema-hash-in-socket-path | Bake the wire-format hash into singleton meta paths so stale binaries go to different paths | ~30 | Trivial |
| **G** | FP reaper sidecar | New `distant-test-reaper` binary in workspace; `FpFixtureLease` test-side struct | ~400 | Medium |
| **H** | Ephemeral host/ssh/docker fixtures | Replace `singleton::start_*` with per-test `MountedHost`/`MountedSsh`/`MountedDocker` using `command-group` + `pdeathsig`/kqueue | ~600 | Medium-High |
| **I** | Test infrastructure simplification | `DistantCmd` builder · `MountedX` fixtures · `assert_mount_status!` macro · log-tail panic hook · mock `MountHandle` in test-harness · dev-fast profile | ~500 | Medium |
| **J** | Coverage gaps | Wire-format frozen fixtures · HLT-01..04 + EVT-01..02 · soak tests · per-backend probe tests · proptest round-trips | ~800 | Medium |
| **K** | nextest profile + diagnostics | Tighter retries · known-flaky `#[ignore]` triage · test result reporter | ~150 | Small |
| **L** | Documentation | TESTING.md updates · CLAUDE.md test author checklist | ~200 | Trivial |

**Total estimated LOC**: ~2730 (delta against current harness:
about +2000 net, since the per-test fixtures replace ~700 LOC
of singleton machinery).


### Phase E — Wire-format hardening (~50 LOC, 1 day)

The single highest-leverage change. Two sub-phases.

#### E1 · `#[serde(other)]` fallback variants on every wire enum

Audit `distant-core/src/net/manager/data/{request,response,event}.rs`
and `distant-core/src/protocol/**` for every `enum` derived with
`Serialize, Deserialize`. For each:

- If it's a unit-variant enum: add an `Unknown` variant marked
  `#[serde(other)]` so unknown variants from a newer peer are
  tolerated as `Unknown` instead of producing a hard
  deserialization error.
- If it's a tagged enum (`#[serde(tag = "type")]`) with struct
  variants: add `Unknown { .. }` with `#[serde(other)]`.
- For variants that legitimately can't be tolerated (the
  `Channel { request: UntypedRequest }` arm in `ManagerRequest`),
  document why and skip.

This single change converts the failure mode "manager rejects
the request silently with a deserialize error" into "manager
sees `Unknown` and returns a typed `UnknownRequest` error" —
something the test framework can act on.

**Acceptance**: a deliberate compile-time-only test that
serializes a fake `Unknown` variant and asserts the round-trip
through every wire enum works. No runtime change at the
protocol level.

#### E2 · Compile-time wire schema hash

Add `distant-core/src/net/manager/schema_hash.rs`:

```rust
/// Compile-time hash of the wire format. Recomputed on every
/// build by hashing the textual representation of the wire-type
/// modules. Any change to a request/response/event variant
/// produces a different hash.
pub const WIRE_SCHEMA_HASH: u64 = const_fnv1a_hash::fnv1a_hash_str_64(
    concat!(
        include_str!("data/request.rs"),
        include_str!("data/response.rs"),
        include_str!("data/event.rs"),
        include_str!("../../protocol/mount.rs"),
    )
);

/// Returns the schema hash as a hex string suitable for embedding
/// in file paths.
pub fn schema_hash_hex() -> String {
    format!("{:016x}", WIRE_SCHEMA_HASH)
}
```

This hash is then used in Phase F by the singleton meta-file
naming scheme.

**Acceptance**: changing one byte in `request.rs` produces a
different `schema_hash_hex()` output.

### Phase F — Schema-hash-in-singleton-path (~30 LOC, 1 day)

Bake the schema hash into singleton meta paths so binaries
built from different wire formats automatically go to different
paths. Stale singletons from old binaries can never silently
serve new requests.

```rust
// distant-test-harness/src/singleton.rs

fn base_path(backend: &str) -> PathBuf {
    let workspace = workspace_hash();
    let schema = distant_core::net::manager::schema_hash_hex();
    std::env::temp_dir().join(format!(
        "distant-test-{workspace}-{schema}-{backend}"
    ))
}
```

That's the entire change. Old singletons live at
`distant-test-<workspace>-<oldhash>-<backend>.meta`; new
binaries look at `distant-test-<workspace>-<newhash>-<backend>.meta`.
The old singleton's `--shutdown lonely=30` timer fires when its
last lease disconnects, and it self-terminates without ever
seeing the new binary's traffic.

This **alone** would have prevented every "wire format mismatch
silently produces empty results" failure I hit during the
session. It is the highest-impact one-line change in the entire
plan.

**Acceptance**: the cross-version compatibility test from the
old plan (which would have built two binaries and asserted no
mixing) becomes unnecessary because the architecture makes
mixing impossible.

### Phase G — FP reaper sidecar (~400 LOC, 2 days)

The macOS FileProvider appex is the **only** fixture that
genuinely cannot be per-test (macOS allows exactly one File
Provider extension instance per bundle ID per machine). For
this case alone, build a Ryuk-style reaper.

#### G1 · `distant-test-reaper` workspace binary

A new binary at `distant-test-harness/src/bin/distant-test-reaper.rs`:

```rust
fn main() -> Result<()> {
    let args = parse_args();
    match args.command {
        Command::Serve => serve(),
        Command::Prune => prune(),
    }
}

fn serve() -> Result<()> {
    let socket_path = reaper_socket_path();  // includes schema hash
    let pid_path = reaper_pid_path();

    // Self-heal: if a stale socket exists, try to connect; if it
    // fails, unlink it and kill any orphan PID from the pidfile.
    self_heal(&socket_path, &pid_path)?;

    let listener = UnixListener::bind(&socket_path)?;
    write_pid_file(&pid_path)?;

    // Spawn the FP fixture (manager + server + appex install)
    let fixture = FpFixture::start()?;

    let lease_count = Arc::new(AtomicUsize::new(0));
    let linger_until = Arc::new(Mutex::new(None));

    spawn_signal_handler(&fixture);
    spawn_linger_watcher(&lease_count, &linger_until, &fixture);

    for conn in listener.incoming() {
        let conn = conn?;
        let count = Arc::clone(&lease_count);
        let linger = Arc::clone(&linger_until);
        let fixture_info = fixture.greeting();
        thread::spawn(move || {
            count.fetch_add(1, Ordering::SeqCst);
            *linger.lock().unwrap() = None;
            // Send fixture greeting (socket path, version)
            let _ = serde_json::to_writer(&conn, &fixture_info);
            let _ = conn.shutdown(Shutdown::Write);
            // Park on the connection until the client closes it
            let mut buf = [0u8; 1];
            let _ = (&conn).read(&mut buf);  // returns 0 on close
            // Lease released
            let prev = count.fetch_sub(1, Ordering::SeqCst);
            if prev == 1 {
                *linger.lock().unwrap() = Some(
                    Instant::now() + Duration::from_secs(LINGER_SECS)
                );
            }
        });
    }
    Ok(())
}
```

Key behaviors:

- **Connection lease lifecycle**: each test that wants the FP
  fixture opens a Unix socket connection to the reaper. The
  connection IS the lease; closing it (any way — graceful, panic,
  SIGKILL of the test) starts a 5-second linger timer.
- **Linger to amortize spawn cost**: if a new test connects
  within 5 seconds of the previous test's last release, the
  fixture is reused. If 5 seconds pass with zero leases, the
  reaper kills the FP fixture and exits. Next test fork-execs a
  fresh reaper.
- **Schema-hash in socket path**: `/tmp/distant-test-reaper-<schema>.sock`.
  Stale reapers from old binaries are on different paths and
  cannot collide with the new tests.
- **Self-heal on startup**: if a socket file exists at the
  reaper's path but `connect()` fails (the previous reaper was
  SIGKILL'd, leaving a stale socket inode), the new reaper
  unlinks the socket file, reads the PID from the sibling pid
  file, and `kill -TERM` it with a 2s grace period before
  `kill -KILL`. Then continues with normal startup.
- **No external state**: everything the reaper needs is in
  `/tmp` next to its socket. No `~/Library/CloudStorage/`
  cleanup, no `~/Applications/Distant.app` cleanup. The FP
  fixture inside the reaper handles its own lifecycle.

#### G2 · `FpFixtureLease` test-side struct

```rust
// distant-test-harness/src/fixtures/fp.rs

pub struct FpFixtureLease {
    socket: UnixStream,
    fixture_info: FpFixtureInfo,
}

impl FpFixtureLease {
    pub fn acquire() -> io::Result<Self> {
        let socket_path = reaper_socket_path();
        for attempt in 0..10 {
            match UnixStream::connect(&socket_path) {
                Ok(socket) => {
                    let fixture_info: FpFixtureInfo =
                        serde_json::from_reader(&socket)?;
                    return Ok(Self { socket, fixture_info });
                }
                Err(_) if attempt < 9 => {
                    Self::ensure_reaper_running()?;
                    std::thread::sleep(Duration::from_millis(100 * (attempt + 1)));
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("FP reaper unreachable after 10 attempts"))
    }

    fn ensure_reaper_running() -> io::Result<()> {
        // Spawn `distant-test-reaper serve` in detached pgid so
        // it outlives the test process.
        let bin = workspace_root().join("target/debug/distant-test-reaper");
        let _ = command_group::CommandGroup::group_spawn(
            std::process::Command::new(&bin).arg("serve")
        )?;
        Ok(())
    }

    pub fn manager_socket(&self) -> &Path {
        &self.fixture_info.manager_socket
    }
}

// On Drop, the UnixStream closes; the reaper sees the close and
// starts its linger timer. No explicit cleanup needed.
```

**Acceptance**: 5 consecutive nextest runs of an FP test with a
SIGKILL between each run all succeed without manual cleanup; no
orphan reapers, no orphan FP fixtures, no manual `pkill`.

### Phase H — Ephemeral host/ssh/docker fixtures (~600 LOC, 3 days)

Replace `singleton::start_host`, `start_ssh`, `start_docker`
with per-test fixtures that own their entire process tree and
clean up on Drop via `command-group`. **This is the architectural
core of the refactor.** It eliminates the singleton model for
the 80% case.

#### H1 · `MountedHost` fixture

```rust
// distant-test-harness/src/fixtures/host.rs

pub struct MountedHost {
    manager: GroupChild,        // command-group::GroupChild
    server: GroupChild,
    socket_path: PathBuf,
    _temp_dir: TempDir,         // owns the socket file
}

impl MountedHost {
    pub fn start() -> io::Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let socket_path = temp_dir.path().join("manager.sock");

        let manager = Self::spawn_manager(&socket_path)?;
        wait_for_socket_ready(&socket_path)?;

        let server = Self::spawn_server()?;
        let credentials = read_credentials(&server)?;

        Self::connect_manager_to_server(&socket_path, &credentials)?;

        Ok(Self { manager, server, socket_path, _temp_dir: temp_dir })
    }

    fn spawn_manager(socket: &Path) -> io::Result<GroupChild> {
        let mut cmd = std::process::Command::new(workspace_distant_bin());
        cmd.args([
            "manager", "listen",
            "--unix-socket", socket.to_str().unwrap(),
            "--shutdown", "lonely=10",  // safety net if Drop is skipped
            "--log-level", "trace",
        ]);

        // SIGKILL coverage: ask the kernel to kill us when our parent dies.
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                nix::sys::prctl::set_pdeathsig(Some(nix::sys::signal::Signal::SIGTERM))
                    .map_err(|e| io::Error::from_raw_os_error(e as i32))
            });
        }
        // On macOS, set up a kqueue parent watcher inside the child.
        // (Implemented via a `--watch-parent <pid>` flag on `distant manager`,
        //  see Phase H4.)
        #[cfg(target_os = "macos")]
        cmd.args(["--watch-parent", &std::process::id().to_string()]);

        // command-group puts the child in its own pgid (Unix) or job object (Windows).
        cmd.group_spawn().map_err(io::Error::from)
    }
}

impl Drop for MountedHost {
    fn drop(&mut self) {
        // killpg the entire process group; reaps grandchildren too.
        let _ = self.manager.kill();
        let _ = self.server.kill();
        let _ = self.manager.wait();
        let _ = self.server.wait();
        // _temp_dir drops, removing the socket file.
    }
}
```

**Spawn cost**: ~100ms on the reference hardware (manager + server +
connect handshake), vs the singleton's amortized 0ms after first
test. Tradeoff: 50 mount tests × 100ms = 5 extra seconds of
test runtime, in exchange for eliminating every cleanup failure
mode.

#### H2 · `MountedSsh` fixture

Same shape as `MountedHost`, but the test owns its own sshd:

```rust
pub struct MountedSsh {
    sshd: Sshd,                 // already exists in test-harness
    manager: GroupChild,
    socket_path: PathBuf,
    _temp_dir: TempDir,
}

impl MountedSsh {
    pub fn start() -> io::Result<Self> {
        let sshd = Sshd::spawn(Default::default())?;
        // ... spawn manager via group_spawn, connect via ssh:// ...
    }
}

impl Drop for MountedSsh {
    fn drop(&mut self) {
        let _ = self.manager.kill();
        let _ = self.manager.wait();
        // sshd::Drop kills the sshd child too
    }
}
```

The existing `Sshd::spawn` already works per-test; we just stop
forgetting it.

#### H3 · `MountedDocker` fixture

Same shape; owns its own container. The container has
`auto_remove = true` so docker daemon cleans it up if Drop is
skipped. `command-group` handles the manager process tree.

#### H4 · `--watch-parent` flag on `distant manager` (and `server`)

The `pdeathsig` mechanism doesn't exist on macOS. Add an opt-in
`--watch-parent <PID>` flag to `distant manager listen` and
`distant server listen` that spawns a thread inside the child
which kqueues `EVFILT_PROC | NOTE_EXIT` on the given PID and
calls `std::process::exit(0)` when the parent dies.

```rust
// distant-core/src/net/server/parent_watcher.rs

#[cfg(target_os = "macos")]
pub fn watch_parent(parent_pid: u32) -> io::Result<()> {
    std::thread::Builder::new()
        .name("parent-watcher".into())
        .spawn(move || {
            use nix::sys::event::*;
            let kq = match kqueue() {
                Ok(kq) => kq,
                Err(_) => return,
            };
            let ev = KEvent::new(
                parent_pid as usize,
                EventFilter::EVFILT_PROC,
                EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                FilterFlag::NOTE_EXIT,
                0, 0,
            );
            let mut out = [KEvent::new(0, EventFilter::EVFILT_USER,
                EventFlag::empty(), FilterFlag::empty(), 0, 0)];
            // TOCTOU: if parent already died, kevent returns ENOENT;
            // exit immediately in that case.
            if kevent(kq, &[ev], &mut [], None).is_err() {
                std::process::exit(0);
            }
            // Block until parent exits.
            let _ = kevent(kq, &[], &mut out, None);
            std::process::exit(0);
        })?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn watch_parent(_parent_pid: u32) -> io::Result<()> {
    // Linux uses pdeathsig in pre_exec; this is a no-op.
    Ok(())
}
```

This is **production-quality code, not test-only**. It belongs
in `distant-core` so production users can also opt into
parent-watching when running distant as a child of a supervisor.

#### H5 · Migrate tests to fixtures

For each of the 37 tests in the inventory's "shared singleton"
and "owned mount" categories:

1. Replace `let ctx = skip_if_no_backend!(backend);` with
   `let host = MountedHost::start()?;`
2. Replace `let sm = mount::get_or_start_mount(&ctx, mount);`
   with `let mp = host.mount(mount, ...).unwrap();`
3. Delete the `unique_subdir` call — every test has its own
   remote root now.
4. Test bodies are otherwise unchanged.

Tests that already use `HostManagerCtx::start()` (HLT-05,
unmount::all_*) just rename to `MountedHost::start()`.

The two FP tests (currently using FP singleton) become
`let fp = FpFixtureLease::acquire()?;`.

**Acceptance**: every test in `tests/cli/mount/**` passes via
the new fixtures with no behavior change. Total wall-clock for
the suite goes UP by ~5s (per-test spawn cost) but the
flakiness goes DOWN to 0.

### Phase I — Test infrastructure simplification (~500 LOC, 2 days)

#### I1 · Typed `DistantCmd` builder

Add `distant-test-harness::cmd::DistantCmd` (covered in old
plan). Compile-time CLI typo catching. The HLT-05 test had two
CLI typos (`manager list`, `client kill`) on first attempt; a
typed builder eliminates that class of bug.

```rust
let mounts: Vec<MountInfo> = DistantCmd::new(&host)
    .status()
    .show(ResourceKind::Mount)
    .format_json()
    .run()
    .expect_success()
    .parse_json::<Vec<MountInfo>>()?;
```

#### I2 · `assert_mount_status!` macro

Wraps the common pattern with full failure context:

```rust
assert_mount_status!(host, |mounts| mounts.iter().any(|m| m.backend == "nfs"));
```

On failure, the macro panic message includes:
- The `MountedHost` (or whatever fixture) diagnostic dump
- The full command line that ran
- The exit code, stdout, stderr
- The last 50 lines of the manager log file (read from
  `host.manager_log_path()`)
- Same for the server log file

Replaces the bare `assert!(stdout.contains(...), "...")` pattern
that hides failure root cause.

#### I3 · Promote `ScriptedMountHandle` to `distant-test-harness::mock`

(Same as old plan I3.) Plus sibling variants `BlockingMountHandle`,
`FailingMountHandle`, `LaggyMountHandle`.

#### I4 · `dev-fast` profile + linker docs

(Same as old plan I4.) `mold`/`lld` setup, `dev-fast` cargo
profile for faster local iteration.

#### I5 · Inline log dump on panic via `panic::set_hook`

Install a process-wide panic hook in
`distant_test_harness::install_test_panic_hook()` that, when a
mount test panics, looks up the active fixture's log files via
a thread-local fixture registry and prints the last 100 lines
of each before letting the default hook run.

### Phase J — Coverage gaps (~800 LOC, 4 days)

#### J1 · Frozen wire-format fixtures

JSON files under `distant-core/src/protocol/fixtures/`. One per
request/response/event variant. A single test loads each and
asserts it round-trips through current types. When the wire
format breaks, the test fails with a diff.

When you intentionally break the wire format (and have updated
all callers), the test fails and prompts you to either bump the
fixture directory (`fixtures/v0.22.0/`) or update the existing
fixtures. This is the canonical way to make breaking changes
visible in PR review.

#### J2 · HLT-01..04 + EVT-01..02

(Deferred from Phase 5 of the previous plan.) Now trivial because
each test owns a `MountedSsh` fixture and can `kill -9` the
sshd directly to simulate connection drops.

```rust
#[rstest]
fn hlt_02_connection_drop_to_disconnected(ssh: MountedSsh) {
    let mp = ssh.mount(MountBackend::Nfs, "/tmp")?;
    ssh.kill_sshd();  // simulates remote disappearance
    poll_until(Duration::from_secs(10), || {
        mp.status() == MountStatus::Disconnected
    }).expect("mount should transition to Disconnected");
}
```

#### J3 · Soak / leak tests

Gated `#[ignore]`, run via `cargo nextest run --run-ignored only`.
For each backend, loop 100 mount/unmount cycles and assert
process count + open FD count stay flat.

#### J4 · Per-backend probe tests

(Deferred from Phase 4 of Network Resilience.) Once granular
per-backend probes are implemented, each backend gets its own
probe-specific test that simulates that backend's failure mode.

#### J5 · Property-based round-trip tests

`proptest` over every wire enum / type. 256 cases per type.

### Phase K — nextest profile + diagnostics (~150 LOC, 1 day)

#### K1 · Tighten `.config/nextest.toml`

- Lower retry count from 4 to 2 for `mount-integration` group.
- Mark known-flaky tests with `#[ignore = "tracking #ISSUE"]`
  until fixed.
- Add `--no-tests warn` so subset filters fail loudly.
- Remove the `leak-timeout = 1s pass` override on
  mount-integration (unnecessary once tests own their process
  tree — leaks should fail).

#### K2 · `scripts/test-report.sh` for CI artifact upload

Parses `cargo nextest run ... --message-format=libtest-json`
output and produces a categorized markdown report (compilation
/ panic / timeout / flaky / leaky). Optional, useful for CI.

### Phase L — Documentation & process (~200 LOC, 1 day)

#### L1 · `docs/TESTING.md` updates

- "Test fixtures: when to use which" section
  (`MountedHost`/`MountedSsh`/`MountedDocker`/`FpFixtureLease`).
- "Diagnosing flaky tests" walkthrough.
- "Why my test sees no mounts" troubleshooting (now mostly
  obsolete after Phase F, but document the schema-hash
  mechanism so future contributors understand it).

#### L2 · CLAUDE.md test author checklist

- [ ] Did you pick the right fixture for your test?
- [ ] Does every assertion include diagnostic context (use
      `assert_mount_status!`)?
- [ ] Did you spawn the test through the test-implementor
      agent and gate it with test-validator?

### Phase ordering & dependencies

```
E (wire format)         ─┬─→ F (schema hash in path)
                         │
F (schema hash in path) ─┴─→ G (FP reaper) [reaper bakes hash into socket path]
                              │
                              ▼
                         G (FP reaper) ─→ H5 (migrate FP tests to lease)

H1, H2, H3 (fixtures)   ──→ H5 (migrate other tests)
H4 (--watch-parent)     ──→ H1, H2, H3 (used by their pre_exec)

I1 (DistantCmd)         ──→ I2 (assert! macro)
I2 (assert! macro)      ──→ H5 (used by migrated tests)
I3 (mock handles)       ──→ independent
I4 (dev-fast)           ──→ independent
I5 (panic hook)         ──→ H5 (panic hook reads fixture log paths)

J1 (wire fixtures)      ──→ E (uses serde's relaxed enums)
J2 (HLT/EVT)            ──→ H2, H4 (need MountedSsh + parent watcher)
J3 (soak)               ──→ H1-H3 (need ephemeral fixtures to count)
J4 (per-backend probe)  ──→ blocked on granular probe impls
J5 (proptest)           ──→ E (uses Unknown variants)

K1 (nextest tweaks)     ──→ H5 (after migration, can drop leak-timeout pass)
K2 (test report)        ──→ independent

L1 (TESTING.md)         ──→ all phases above
L2 (checklist)          ──→ all phases above
```

**Execution order**: E → F → G → H → I → J → K → L. Phases
within a letter are mostly independent. Phase H is the largest
and has the most rippling effects; if blocked, fall back to
F + G + I + J5 as a smaller "fix the worst stuff first" slice.

### What we DROP from the previous draft

This refactor explicitly DROPS the following old-plan items
because they patch the wrong layer:

- ❌ `scripts/test-mount-clean.sh` — not needed once fixtures
  self-clean
- ❌ `scripts/test-mount-preflight.sh` — same
- ❌ `scripts/test-report.sh` (downgraded to optional Phase K2)
- ❌ Build-hash validation in singleton meta files (replaced by
  schema-hash-in-path, which is structural)
- ❌ `MountSingletonScope::Owned` opt-in (no singletons to scope)
- ❌ PID-locked sentinels (no singletons to lock)
- ❌ Cross-version compatibility test (made impossible by Phase F)
- ❌ FP domain bulk reset (handled by reaper self-heal)

### What we KEEP from the previous draft

- ✅ `assert_mount_status!` macro (Phase I2)
- ✅ Inline log tail panic hook (Phase I5)
- ✅ Mock `MountHandle` in test-harness (Phase I3)
- ✅ Wire format frozen fixtures (Phase J1)
- ✅ HLT-01..04 + EVT-01..02 (Phase J2)
- ✅ Soak tests (Phase J3)
- ✅ Per-backend probe tests (Phase J4)
- ✅ proptest round-trips (Phase J5)
- ✅ nextest profile tweaks (Phase K1)
- ✅ TESTING.md + CLAUDE.md updates (Phase L1, L2)
- ✅ `dev-fast` profile (Phase I4)
- ✅ `DistantCmd` builder (Phase I1)

### Lessons-learned reconciliation

The original "Lessons from Phase 0–6" section enumerated 9
incidents during the rollout. Each is addressed as follows:

| Incident | Old plan addressed via | New plan addresses via |
|---|---|---|
| Stale singleton state was the #1 friction source | E1 cleanup script + E2 build-hash | F (schema-hash-in-path) — **structural** |
| "No mounts found" was uninformative | F1 diagnostic macro | I2 `assert_mount_status!` (same idea) |
| Test harness compilation fragile under feature subsets | (not addressed) | (not addressed; orthogonal) |
| Cherry-pick conflict resolution lossy | (not addressed) | (not addressed; orthogonal) |
| Tests didn't catch the orphan-mount latent bug | H5 per-backend probe tests | J4 (kept) |
| Background tasks vs foreground tasks vs timeouts | J2 preflight script | (not addressed; usage discipline) |
| Build cycle is 10–30s of latency | I4 dev-fast profile | I4 (kept) |
| Test author boilerplate too high | I1 DistantCmd, I2 fixtures | I1 (kept) + H (which moves boilerplate INTO the fixture types so test bodies are tiny) |
| Flakes masked by retries | J1 nextest tweaks | K1 (kept), strengthened by H eliminating shared state as flake source |

### Validation checklist

When this entire plan has been executed:

```bash
# A. Repeated runs are idempotent — no manual cleanup ever
for i in 1 2 3 4 5; do
    cargo nextest run --all-features -p distant -E 'test(mount::)' || exit 1
done

# B. SIGKILL recovery — the next run cleans up automatically
cargo nextest run --all-features -p distant -E 'test(mount::)' &
sleep 5
kill -9 $!
sleep 2
# No manual pkill / rm -f required:
cargo nextest run --all-features -p distant -E 'test(mount::)'

# C. Wire format mismatch is impossible
git stash
git checkout HEAD~10 -- distant-core/src/protocol/mount.rs  # break wire fmt
cargo build
# This run uses the OLD binary's schema hash; it spawns a separate reaper.
cargo nextest run --all-features -p distant -E 'test(host_fp)'
git checkout HEAD -- distant-core/src/protocol/mount.rs
git stash pop
cargo build
# This run uses the NEW binary's schema hash; spawns a separate reaper.
# The OLD reaper is still running but on a different socket path; it
# self-terminates after the lonely-shutdown timer.
cargo nextest run --all-features -p distant -E 'test(host_fp)'

# D. cargo test still works
cargo test --all-features -p distant mount::

# E. SIGINT (Ctrl+C) leaves no orphans
cargo nextest run --all-features -p distant -E 'test(mount::)' &
sleep 5
kill -INT $!
sleep 5
ps aux | grep -i distant  # should be empty
```

If all five scenarios pass without manual intervention, the
refactor is complete.
