# Mount Backends — Production Fixes & Full Test Coverage PRD

## Status (2026-04-06)

**228/228 mount tests passing (100%).** All 37 FP tests now pass after
the FP fix series (commits `9f0c834` → `86d794d`). Suite is fully green
on macOS. The next phase of work — network resilience + mount health
monitoring — is described in
[§ Plan: Network Resilience + Mount Health](#plan-network-resilience--mount-health)
below.

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

### Remaining (becomes the new plan)

- [ ] Health monitoring: periodic checks per backend type (Phase 5)
- [ ] Connection drop → mount "disconnected" → reconnect → resume (Phase 5)
- [ ] Process audit: expect ~5 distant processes (vs 30+ today) (Phase 6)
- [ ] Windows testing via ssh windows-vm + rsync + cargo nextest
- [ ] Generic event subscription system (incorporated from PR #288)

See [PROGRESS.md](PROGRESS.md) for the detailed checklist and
[§ Plan: Network Resilience + Mount Health](#plan-network-resilience--mount-health)
below for the full step-by-step.

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
