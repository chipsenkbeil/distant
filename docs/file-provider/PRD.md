# macOS File Provider — Product Requirements Document

## Overview

The macOS File Provider integration exposes remote filesystems mounted via
`distant connect` as native Finder locations through Apple's
[FileProvider framework](https://developer.apple.com/documentation/fileprovider).
Users see a sidebar entry in Finder and can browse, open, edit, and save files
as if they were local, with content fetched on demand from the remote server.

### Current State

The structural implementation exists: a `.appex` extension is bundled inside
`Distant.app`, the same binary serves both CLI and extension roles, domain
registration and IPC via App Group shared container are wired up. However,
**Finder shows "Loading..." forever** when opening a mounted domain. The
extension launches and enters `dispatch_main` but never successfully serves
content to `fileproviderd`.

### Goal

A working File Provider that can browse directories, open files, create/edit
files, and delete items on a remote server — all through native Finder UI.

---

## Architecture Summary

```
Finder ──XPC──▶ fileproviderd ──launches──▶ DistantFileProvider.appex
                                                  │
                                            Unix socket (App Group)
                                                  │
                                            distant manager daemon
                                                  │
                                            SSH/Docker/etc channel
                                                  │
                                            remote server
```

**Key files:**

| File | Role |
|------|------|
| `distant-mount/src/backend/macos_file_provider.rs` | Global state, bootstrap, handler functions, domain management |
| `distant-mount/src/backend/macos_file_provider/provider.rs` | `DistantFileProvider` ObjC class (NSFileProviderReplicatedExtension) |
| `distant-mount/src/backend/macos_file_provider/provider/enumerator.rs` | `DistantFileProviderEnumerator` ObjC class (NSFileProviderEnumerator) |
| `distant-mount/src/backend/macos_file_provider/provider/enumerator/item.rs` | `DistantFileProviderItem` ObjC class (NSFileProviderItemProtocol) |
| `distant-mount/src/backend/macos_file_provider/utils.rs` | App Group container path, bundle detection |
| `distant-mount/src/core/runtime.rs` | Async-to-sync bridge (`Runtime::spawn`) |
| `distant-mount/src/core/remote.rs` | `RemoteFs` — translates FS ops to distant protocol |
| `src/macos_appex.rs` | Extension entry point: logging, Tokio RT, channel resolver |
| `src/cli/commands/client.rs` | CLI `mount`/`unmount` commands |
| `resources/macos/Extension-Info.plist` | Appex bundle config |
| `scripts/make-app.sh` | Build + bundle + sign + install pipeline |
| `scripts/logs-appex.sh` | Diagnostic log viewer |

---

## Phase 0: Diagnostics & Observability

**Goal:** Be able to see what the extension is doing so we can debug failures.

### P0.1 — Fix log file location in `logs-appex.sh`

The script looks in `~/Library/Containers/dev.distant.file-provider/Data/tmp/`
but the current code writes to
`~/Library/Group Containers/39C6AGD73Z.group.dev.distant/logs/`. Update the
script to check both locations (for backward compat with old builds).

### P0.2 — Add structured logging to all ObjC entry points

Every `define_class!` method must log entry/exit with parameters:
- `initWithDomain:` — domain identifier, display name
- `invalidate` — which domain
- `enumeratorForContainerItemIdentifier:` — container ID (distinguish root,
  working set, trash, numeric inode)
- `itemForIdentifier:` — identifier string
- `fetchContentsForItemWithIdentifier:` — identifier
- `createItemBasedOnTemplate:` — filename, parent, has_content
- `modifyItem:` — identifier, has new contents
- `deleteItemWithIdentifier:` — identifier
- `enumerateItemsForObserver:` — container, page
- `enumerateChangesForObserver:` — container, anchor
- `currentSyncAnchorWithCompletionHandler:` — (entry only)

Log bootstrap success/failure with specific error messages. Log channel
resolver connection attempts and outcomes.

### P0.3 — Add a `distant mount status` subcommand

Query registered FileProvider domains via `NSFileProviderManager::getDomainsWithCompletionHandler`
and display:
- Domain identifier and display name
- Whether metadata file exists in `domains/`
- Manager daemon reachability (can we connect to the App Group socket?)
- Connection ID validity (is the connection still alive?)

---

## Phase 1: Show Root Directory (Critical Path)

**Goal:** Opening the Finder sidebar entry shows the root directory listing.

### P1.1 — Handle working set container identifier

`enumeratorForContainerItemIdentifier:` receives
`NSFileProviderWorkingSetContainerItemIdentifier` as the first request from
`fileproviderd`. The enumerator must:
- Detect this identifier (compare against the framework constant)
- In `enumerateItems`: call `finishEnumeratingUpToPage(nil)` with zero items
  (the working set starts empty for a remote FS)
- In `enumerateChanges`: call
  `finishEnumeratingChangesUpToSyncAnchor_moreComing` with current anchor and
  `moreComing=false`

**Without this, `fileproviderd` hangs waiting for a valid working set response
before it will enumerate the root.**

### P1.2 — Handle trash container identifier

`enumeratorForContainerItemIdentifier:` may receive
`NSFileProviderTrashContainerItemIdentifier`. Either:
- Return an empty enumerator (same pattern as working set), OR
- Write `NSFeatureUnsupportedError` to the error out-parameter and return nil

### P1.3 — Handle root container identifier in `itemForIdentifier`

When `itemForIdentifier:` receives `NSFileProviderRootContainerItemIdentifier`:
- Return an item with `itemIdentifier` = the root constant (not "1")
- `parentItemIdentifier` = the root constant
- `filename` = "" or the domain display name
- `contentType` = `UTTypeFolder`
- `capabilities` = Read + ContentEnumerating

Currently the code parses the identifier as u64 with `unwrap_or(1)` — this
must be replaced with explicit constant matching.

### P1.4 — Map root container identifier in enumerator

In `enumerateItemsForObserver:`, when the container is
`NSFileProviderRootContainerItemIdentifier`, enumerate inode 1 (the root).
Child items must set `parentItemIdentifier` to the root constant string, not
"1".

### P1.5 — Ensure bootstrap succeeds and is visible

Add logging that confirms:
1. Domain metadata file was found and parsed
2. Channel resolver connected to the manager
3. `RemoteFs` initialization completed (the `watch::Receiver` received `true`)

If bootstrap fails, `enumerateItems` should call `finishEnumeratingWithError`
(not `finishEnumeratingUpToPage(nil)`) so Finder shows an error instead of
an empty/loading state.

### P1.6 — Rebuild and verify

After each change:
```bash
./scripts/make-app.sh
/Applications/Distant.app/Contents/MacOS/distant connect ssh://target
/Applications/Distant.app/Contents/MacOS/distant mount
# Open Finder sidebar → check if root directory appears
# Check logs: scripts/logs-appex.sh
/Applications/Distant.app/Contents/MacOS/distant unmount --all
```

---

## Phase 2: Browse Subdirectories & Open Files

**Goal:** Navigate into subdirectories and open files from Finder.

### P2.1 — Fix `itemForIdentifier` for numeric inode identifiers

When the identifier is a numeric string (e.g., "42"), look up the inode via
`RemoteFs::getattr`. Return an `NSFileProviderItem` with:
- `itemIdentifier` = the numeric string
- `parentItemIdentifier` = parent inode string (or root constant if parent is
  root)
- Correct `filename`, `contentType`, `size`, `itemVersion`

### P2.2 — Fix parent identifier in enumerated items

When `enumerateItems` builds child items, if the parent inode is 1 (root), use
`NSFileProviderRootContainerItemIdentifier` as the parent identifier string,
not "1".

### P2.3 — Verify subdirectory navigation

Navigate 2-3 levels deep in Finder. Verify that:
- Each level loads and shows correct contents
- Going back to parent works
- Item counts match `ls` output on the remote

### P2.4 — Verify file opening

Double-click a text file in Finder. Verify:
- `fetchContentsForItemWithIdentifier:` is called (check logs)
- A temp file is created at `/tmp/distant_fp_<ino>`
- The file opens in the default editor with correct content
- The temp file URL returned is valid and accessible

---

## Phase 3: Write Operations

**Goal:** Create, modify, and delete files/directories through Finder.

### P3.1 — Fix `modifyItem` parent identifier

In `handle_modify_item` (macos_file_provider.rs:475), the parent identifier is
hardcoded to `"1"`. Look up the actual parent inode from `RemoteFs::get_path`
and use it (or the root constant if parent is root).

### P3.2 — Fix `createItem` to handle file content

`handle_create_item` receives a `url` parameter for file content but only
checks `has_content` as bool. When `url` is `Some`, read the local file at
that URL and write it to the remote after creating the file.

### P3.3 — Verify create operations

- Create a new folder via Finder (Cmd+Shift+N)
- Create a new file (drag a file into the mount)
- Verify items appear on the remote

### P3.4 — Verify modify operations

- Edit a text file, save it
- Verify content is updated on the remote
- Verify Finder shows updated modification date

### P3.5 — Verify delete operations

- Delete a file via Finder (Cmd+Delete)
- Delete a folder
- Verify items are removed on the remote

---

## Phase 4: Robustness & Edge Cases

**Goal:** Handle real-world usage patterns without crashes or hangs.

### P4.1 — Streamed file reads for large files

Replace `fs.read(ino, 0, u32::MAX)` in `handle_fetch_contents` with chunked
reads. Write chunks to the temp file incrementally instead of buffering the
entire file in memory.

### P4.2 — Per-domain Runtime (multi-mount support)

Replace the global `OnceLock<Arc<Runtime>>` with a `Mutex<HashMap<String, Arc<Runtime>>>`
keyed by domain identifier. This allows multiple simultaneous mounts (e.g.,
two SSH connections) in the same appex process.

### P4.3 — Graceful bootstrap failure

If the manager daemon is unreachable or the connection ID is stale:
- Signal the error to `fileproviderd` via `finishEnumeratingWithError`
- Use `NSFileProviderError::serverUnreachable` (not generic NSCocoaError)
- Finder shows a meaningful error instead of "Loading..."

### P4.4 — Domain display name includes connection info

Currently the display name is `ssh-root@host`. Include a "Distant" prefix
so Finder shows `Distant — ssh-root@host` (or similar) to make it clear
this is a distant mount.

### P4.5 — Handle `.` and `..` in readdir

The enumerator already filters these, but verify the filter works correctly
on all remote server types (Linux, Windows via SSH, Docker containers).

### P4.6 — Handle symlinks

Currently symlinks appear as regular files. Either:
- Resolve symlinks and present the target type, OR
- Skip symlinks in enumeration with a log message

---

## Phase 5: Change Notifications & Performance

**Goal:** Keep Finder in sync with remote changes and improve responsiveness.

### P5.1 — Signal changes via `NSFileProviderManager::signalEnumerator`

When `RemoteFs`'s watch task detects a change (via the distant `watch`
protocol), call `NSFileProviderManager::signalEnumeratorForContainerItemIdentifier`
for the affected directory. This triggers `fileproviderd` to re-enumerate.

### P5.2 — Implement meaningful `enumerateChanges`

Instead of immediately finishing with an empty changeset, track modifications
since the last sync anchor. Return changed items to the observer.

### P5.3 — Pagination for large directories

Implement `startingAtPage` handling in `enumerateItems` to avoid loading
thousands of items in a single call. Use pages of ~100 items.

### P5.4 — Progress tracking for long operations

Return meaningful `NSProgress` objects from `fetchContents` and `modifyItem`
that reflect actual download/upload progress and support cancellation.

### P5.5 — Cache warming on mount

When a domain is first mounted, pre-enumerate the root directory so the first
Finder open is instant rather than waiting for a round-trip.

---

## Phase 6: Polish & Production Readiness

**Goal:** Ready for real users.

### P6.1 — Reconnection on connection loss

If the SSH session drops, the appex should detect the failure and either:
- Attempt reconnection via the manager daemon
- Signal `NSFileProviderError::serverUnreachable` so Finder shows offline state

### P6.2 — Cleanup on unmount

`distant unmount` should:
- Remove the FileProvider domain
- Clean up metadata files
- Clean up any temp files in `/tmp/distant_fp_*`

### P6.3 — Multiple mount identification

Each mount should show a distinguishing name in Finder's sidebar (e.g.,
`windows-vm` instead of generic `Distant`).

### P6.4 — Integration tests

Add tests that:
- Register and remove domains programmatically
- Verify metadata file round-trip (write + read)
- Mock the ObjC callback chain for unit testing

### P6.5 — Documentation

- Update `docs/ARCHITECTURE.md` with File Provider section
- Add `docs/FILE_PROVIDER.md` user guide
- Document the build/sign/install workflow

---

## Non-Goals (Explicit Exclusions)

- **Offline mode / full sync**: This is a cloud-like on-demand provider, not a
  sync engine. Files are fetched when opened, not pre-synced.
- **Conflict resolution**: Single-writer model — last write wins. No merge UI.
- **Thumbnails / Quick Look**: Not in scope for initial implementation.
- **iOS/iPadOS support**: macOS only.
- **Finder tags / favorites**: Not in scope.

---

## Success Criteria

| Milestone | Criteria |
|-----------|----------|
| Phase 0 | Can see extension logs; `distant mount status` shows domain health |
| Phase 1 | Opening mount in Finder shows root directory listing |
| Phase 2 | Can navigate subdirectories and open files |
| Phase 3 | Can create, edit, and delete files via Finder |
| Phase 4 | Large files work; multiple mounts work; errors shown to user |
| Phase 5 | Remote changes appear in Finder; large directories paginated |
| Phase 6 | Survives connection drops; clean unmount; documented |
