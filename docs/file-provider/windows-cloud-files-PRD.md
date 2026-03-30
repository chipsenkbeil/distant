# Windows Cloud Files API — Product Requirements Document

## Overview

Implement a functioning Windows Cloud Files (Cloud Filter API) mount backend
for distant. When a user runs `distant mount --backend windows-cloud-files
C:\Users\...\CloudMount`, the Cloud Filter API provides a native Windows
Explorer-integrated mount of the remote distant filesystem. Files appear as
cloud placeholders and are hydrated on demand.

## Background

A prior attempt used the experimental `cloud-filter` crate (v0.0.6), which
proved unworkable:

- `CfCreatePlaceholders` on the sync root returned `0x8007017C`
  (`ERROR_CLOUD_FILE_INVALID_REQUEST`) consistently
- The crate's `FetchPlaceholders` ticket uses
  `DISABLE_ON_DEMAND_POPULATION` unconditionally
- Internal fields are `pub(crate)`, forcing unsafe casts to access
  connection/transfer keys for direct `CfExecute` calls
- 5 commits of debugging produced no resolution

**Decision:** Rewrite using the `windows` crate directly (targeting 0.62.x),
dropping the `cloud-filter` dependency entirely. This mirrors the macOS File
Provider approach where `objc2` bindings are used directly.

## Architecture

```
  User / Explorer
       |
  cldflt.sys (Cloud Filter minifilter driver)
       |
  [Callback thread pool]
       |
  CloudFilesProvider          -- owns CF_CONNECTION_KEY, dispatches callbacks
       |                         bridges sync callbacks to async via Runtime
  Runtime (core/runtime.rs)   -- tokio Handle + OnceCell<Arc<RemoteFs>>
       |
  RemoteFs (core/remote.rs)   -- translates FS ops to distant protocol calls
       |
  Channel (distant-core)      -- network transport to distant server
```

### Threading Model

Cloud Filter callbacks arrive on arbitrary thread pool threads. They are
**synchronous** — the callback function must not return until the operation
is complete (or a 60-second timeout fires). The `Runtime::spawn()` method
runs the async operation on the tokio handle and blocks the callback thread
via `Handle::block_on()` to get the result.

This differs from the macOS FileProvider (fire-and-forget with completion
handlers) but is compatible with the `Runtime` abstraction.

### Key Design Decisions

1. **Use `windows` crate 0.62.x directly** — no wrapper crates
2. **On-demand population** — use `FETCH_PLACEHOLDERS` callbacks instead of
   recursive pre-population. Register with `PopulationType::Full` so the
   platform requests all entries in a directory when first accessed.
3. **Progressive hydration** — use `HydrationType::Progressive` so partial
   reads complete before full hydration finishes. The prior attempt used
   `Full` which blocks all I/O until the entire file downloads.
4. **Unique sync root IDs** — incorporate mount point path hash into the
   sync root ID to support multiple concurrent mounts.
5. **Fully async** — no `block_on` on the main thread. Callback threads
   use `Handle::block_on()` which is acceptable (they're dedicated threads
   managed by cldflt.sys, not the tokio pool).

## Requirements

### Phase 0: Foundation — Drop `cloud-filter`, Direct API Bindings

> **Goal:** Compiles on Windows with the `windows` crate. No functionality
> yet — just the scaffolding and type definitions.

#### P0.1 — Update Cargo.toml dependencies

Replace `cloud-filter = "0.0.6"` with `windows = "0.62"` (or latest 0.62.x).
Required features:

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.62", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_Storage_CloudFilters",
    "Win32_Storage_FileSystem",
    "Win32_System_Com",
] }
```

#### P0.2 — Define Rust wrapper types

Create thin wrapper types for Cloud Filter API concepts:

- `SyncRootRegistration` — wraps `CfRegisterSyncRoot` / `CfUnregisterSyncRoot`
- `SyncRootConnection` — wraps `CfConnectSyncRoot` / `CfDisconnectSyncRoot`,
  holds the `CF_CONNECTION_KEY`
- `PlaceholderInfo` — wraps `CF_PLACEHOLDER_CREATE_INFO` construction
- Helper for `CfExecute` calls (operation info + parameters)

These should be clean, safe Rust APIs over the raw Win32 functions.

#### P0.3 — Skeleton `CloudFilesProvider` struct

Replace the `cloud-filter`-based `CloudFilesHandler` with a new
`CloudFilesProvider` struct that holds:

- `Arc<RemoteFs>` (shared remote filesystem)
- `mount_point: PathBuf`
- `connection_key: CF_CONNECTION_KEY` (set after connect)

Define the callback function signatures matching `CF_CALLBACK_TYPE`.

### Phase 1: Core Lifecycle — Register, Connect, Disconnect

> **Goal:** `distant mount --backend windows-cloud-files C:\path` registers
> a sync root, connects with callbacks, and `distant unmount` cleanly
> disconnects and unregisters. Explorer shows the folder with cloud overlay
> but no files yet.

#### P1.1 — Sync root registration

Implement `CfRegisterSyncRoot` with:

- Provider name: `"distant"`
- Provider version: from `env!("CARGO_PKG_VERSION")`
- Hydration policy: `CF_HYDRATION_POLICY_PROGRESSIVE`
- Population policy: `CF_POPULATION_POLICY_FULL`
- In-sync policy: `CF_INSYNC_POLICY_TRACK_ALL`
- Sync root identity: mount-point-specific (hash of mount path + connection
  ID) to support multiple mounts
- Icon: `%SystemRoot%\system32\imageres.dll,197` (cloud icon)
- Path: the user-specified mount point

Handle idempotent re-registration: if already registered for this mount
point, unregister first, clean stale reparse points, then re-register.

#### P1.2 — Sync root connection with callback table

Implement `CfConnectSyncRoot` with callback registrations for:

- `CF_CALLBACK_TYPE_FETCH_DATA` (required)
- `CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS` (required)
- `CF_CALLBACK_TYPE_NOTIFY_DELETE` (for remote propagation)
- `CF_CALLBACK_TYPE_NOTIFY_RENAME` (for remote propagation)
- `CF_CALLBACK_TYPE_CANCEL_FETCH_DATA` (graceful cancellation)

Connect flags should include `CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO` and
`CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH`.

Store the returned `CF_CONNECTION_KEY` in the provider.

#### P1.3 — Clean disconnect and unregister

`CfDisconnectSyncRoot` on shutdown signal, then `CfUnregisterSyncRoot`.
Integrate with `MountHandle` shutdown channel. On drop, disconnect if
still connected.

#### P1.4 — CLI integration

Wire into `lib.rs::mount()` match arm for `MountBackend::WindowsCloudFiles`.
Verify `distant mount --backend windows-cloud-files C:\path` succeeds and
Explorer shows the folder.

### Phase 2: Directory Enumeration — Placeholder Population

> **Goal:** `dir C:\CloudMount` shows files from the remote server.
> Navigating into subdirectories triggers on-demand population.

#### P2.1 — FETCH_PLACEHOLDERS callback

When the platform requests directory contents:

1. Extract the directory path from `CF_CALLBACK_INFO::NormalizedPath`
2. Resolve to a remote path relative to the mount root
3. Call `RemoteFs::readdir()` via `Runtime::spawn()` (blocking the
   callback thread)
4. Build `CF_PLACEHOLDER_CREATE_INFO` array from directory entries:
   - `RelativeFileName`: entry name
   - `FsMetadata`: file size, attributes (`FILE_ATTRIBUTE_DIRECTORY` for
     dirs), creation/modification times
   - `FileIdentity`: relative path as UTF-8 bytes (used in subsequent
     callbacks to identify the file)
   - Flags: `CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC`
5. Call `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS`

Filter out `.` and `..` entries.

#### P2.2 — Root directory initial population

After `CfConnectSyncRoot`, trigger initial root directory population
so that `dir` shows files immediately without waiting for Explorer to
trigger `FETCH_PLACEHOLDERS`.

Two approaches (try in order):
1. Call `CfCreatePlaceholders` for root entries after connect
2. If that fails (0x8007017C), rely purely on `FETCH_PLACEHOLDERS` callback

#### P2.3 — Nested directory traversal

When a user enters a subdirectory placeholder, the platform fires
`FETCH_PLACEHOLDERS` for that directory. The callback must resolve the
directory's remote path and populate its children. This should work
automatically from P2.1 if `FileIdentity` stores the relative path.

### Phase 3: File Hydration — Read Access

> **Goal:** `type C:\CloudMount\file.txt` displays file contents.
> Files hydrate on demand and are cached locally by the Cloud Filter driver.

#### P3.1 — FETCH_DATA callback

When the platform requests file data:

1. Extract `FileIdentity` (the relative path set during placeholder creation)
2. Read `RequiredFileOffset` and `RequiredLength` from callback parameters
3. Call `RemoteFs::read()` via `Runtime::spawn()`
4. Call `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_DATA`:
   - Buffer: the read data
   - Offset: the required offset
   - Length: actual bytes read
   - CompletionStatus: `STATUS_SUCCESS` or `NTSTATUS` error code

#### P3.2 — Chunked transfer for large files

For files larger than 4MB, transfer data in chunks (e.g., 1MB at a time)
to avoid memory pressure and to report progress:

- Call `CfExecute(TRANSFER_DATA)` per chunk
- Call `CfReportProviderProgress` between chunks

#### P3.3 — CANCEL_FETCH_DATA callback

When a hydration is cancelled (user closes file, process killed):

- Record the cancellation in a per-file state map
- The next `RemoteFs::read()` result for that file is discarded

### Phase 4: Write Operations — Remote Propagation

> **Goal:** Create, modify, and delete files in the mount and have changes
> propagate to the remote server.

#### P4.1 — NOTIFY_DELETE callback

When a user deletes a file/directory in the mount:

1. Extract the relative path from `FileIdentity` or `NormalizedPath`
2. Determine file vs directory
3. Call `RemoteFs::unlink()` or `RemoteFs::rmdir()` via `Runtime::spawn()`
4. Respond with `CfExecute(ACK_DELETE)` with `STATUS_SUCCESS` to allow
   the deletion, or an error NTSTATUS to block it

#### P4.2 — NOTIFY_RENAME callback

When a user renames/moves a file:

1. Extract source path from `SourcePath` in callback parameters
2. Extract destination path from `NormalizedPath` in callback info
3. Call `RemoteFs::rename()` via `Runtime::spawn()`
4. Respond with `CfExecute(ACK_RENAME)`

#### P4.3 — New file creation (write-back)

When a user creates a new file in the mount:

- Watch for new non-placeholder files via directory watcher
  (`ReadDirectoryChangesW`)
- Read the file content
- Call `RemoteFs::create()` + `RemoteFs::write()` to create on remote
- Convert the local file to a placeholder via `CfConvertToPlaceholder`
- Mark in-sync

#### P4.4 — File modification (write-back)

When a user modifies a hydrated file:

- Detect modification via directory watcher or dehydration notification
- Read the modified content
- Call `RemoteFs::write()` to update remote
- Mark in-sync via `CfSetInSyncState`

### Phase 5: Multiple Mounts & Status

> **Goal:** Support multiple concurrent mount points, including from
> different connections, with proper status reporting and selective unmount.

#### P5.1 — Unique sync root IDs per mount

Each mount gets a unique sync root ID based on:
`distant!{UserSID}!{hash(mount_point + connection_id)}`

This prevents multiple mounts from clobbering each other's registration.

#### P5.2 — Mount status detection

Implement `mount-status` support for Cloud Files mounts:

- Enumerate registered sync roots via `CfGetSyncRootInfoByPath` or
  registry enumeration
- Report: mount point, connection ID, sync state, provider status

#### P5.3 — Selective unmount

`distant unmount C:\CloudMount` should:

1. Disconnect the specific sync root connection
2. Unregister only that sync root
3. Leave other Cloud Files mounts and other backend mounts intact

#### P5.4 — Unmount all

`distant unmount --all` should enumerate and unmount all distant Cloud
Files sync roots in addition to NFS/FUSE/FileProvider mounts.

## Success Criteria

Directly from PLAN.md — all must pass on the Windows 11 VM:

1. **Mount and browse:**
   - `dir` shows root-level files of remote cwd
   - Can create a text file with content; it appears on remote
   - Can delete the text file; it disappears on remote
   - Can traverse into subdirectories

2. **Multiple mounts with `--remote-root`:**
   - Two mounts with different `--remote-root` values don't clobber each other

3. **Multiple connections:**
   - Mounts from different `distant connect` sessions coexist

4. **Mount status:**
   - `distant mount-status` lists Cloud Files mounts

5. **Selective unmount:**
   - Unmounting one mount doesn't affect others

6. **Unmount all:**
   - `distant unmount --all` removes all Cloud Files mounts

## Development Workflow

All development happens on the Mac laptop; testing on the Windows 11 VM.

```bash
# Sync code to VM
rsync -avz \
    --exclude target/ \
    --exclude .git/ \
    /Users/senkwich/projects/distant/ \
    windows-vm:/cygdrive/c/Users/senkwich/Projects/distant/

# Build on VM
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && cargo build"

# Run on VM (after starting a distant server somewhere)
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && \
    target/debug/distant.exe connect distant://:<key>@<host>:<port>"
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && \
    target/debug/distant.exe mount --backend windows-cloud-files \
    C:\\Users\\senkwich\\CloudMount"

# Verify
ssh windows-vm "dir C:\\Users\\senkwich\\CloudMount"
```

**Never create commits from the loop** — all changes live on the Mac,
synced via rsync, tested via SSH.

## Non-Goals

- Shell extensions (thumbnails, context menus, custom state icons) — these
  require COM registration and MSIX packaging, which is out of scope
- Windows Search indexer integration
- Automatic conflict resolution
- Offline file access (files require active distant connection)
- Desktop Bridge / MSIX packaging
- Performance tuning (cache TTLs, prefetch strategies)

## Dependencies

- `windows` crate 0.62.x with `Win32_Storage_CloudFilters` feature
- Windows 10 1709+ (Fall Creators Update) with NTFS volume
- Tokio runtime (already available via `distant-core`)

## Risk Factors

1. **Cloud Filter API is complex** — the callback model with 60s timeouts,
   connection keys, and transfer keys has many subtle requirements
2. **No Rust ecosystem precedent** — the only Rust wrapper (`cloud-filter`)
   is experimental and broken for our use case
3. **NTFS-only** — won't work on ReFS, FAT32, or network drives
4. **Per-directory population state** — NTFS reparse points track population
   state; stale state causes `0x8007017C` errors
5. **Write-back complexity** — detecting local modifications and syncing
   back requires a directory watcher, which is a significant addition
