# Windows Cloud Files API — Implementation Progress

> Auto-updated by the `/file-provider-loop` command (Windows Cloud Files variant).
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 0: Foundation

- [x] **P0.1** Update Cargo.toml dependencies
  - Replaced `cloud-filter = "0.0.6"` with `windows = "0.62"` + features:
    Win32_Foundation, Win32_Security, Win32_Storage_CloudFilters,
    Win32_Storage_FileSystem, Win32_System_Com
  - Removed `cloud-filter` from feature flag and dependencies
  - Files: `distant-mount/Cargo.toml`

- [x] **P0.2** Define Rust wrapper types for Cloud Filter API
  - Deferred full wrapper types to Phase 1 — skeleton uses direct API stubs
  - Global state via `OnceLock` statics (TOKIO_HANDLE, REMOTE_FS, MOUNT_POINT)
    matching the macOS FileProvider pattern
  - Helper: `relative_path()`, `build_sync_root_id()`
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P0.3** Skeleton `CloudFilesProvider` struct
  - Complete rewrite of `windows_cloud_files.rs` removing all `cloud-filter` usage
  - 5 callback stubs: on_fetch_data, on_cancel_fetch_data, on_fetch_placeholders,
    on_notify_delete, on_notify_rename
  - Public API preserved: `mount()`, `pre_populate()`, `unmount()`
  - lib.rs mount match arm unchanged (API-compatible)
  - Compiles clean on macOS (cfg(windows) gated)
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

---

## Phase 1: Core Lifecycle

- [x] **P1.1** Sync root registration
  - `CfRegisterSyncRoot` with full hydration, full population
  - Idempotent: unregister stale roots, clean reparse points, re-register
  - Sync root ID: `distant!default` (Phase 5 adds uniqueness)
  - CF_REGISTER_FLAG_UPDATE for safe re-registration
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P1.2** Sync root connection with callback table
  - `CfConnectSyncRoot` with 5 callbacks + NONE terminator
  - Callbacks: FETCH_DATA, CANCEL_FETCH_DATA, FETCH_PLACEHOLDERS,
    NOTIFY_DELETE, NOTIFY_RENAME (all stubs — Phase 2/3/4 implement)
  - Connect flags: REQUIRE_PROCESS_INFO | REQUIRE_FULL_FILE_PATH
  - `ConnectionGuard` wraps CF_CONNECTION_KEY, disconnects on drop
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P1.3** Clean disconnect and unregister
  - `ConnectionGuard::drop()` calls `CfDisconnectSyncRoot` with logging
  - `unmount()` calls `CfUnregisterSyncRoot` using stored MOUNT_POINT
  - Integrates with existing `MountHandle` shutdown channel in lib.rs
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P1.4** CLI integration verification
  - `distant mount --backend windows-cloud-files C:\path` succeeds
  - `fsutil reparsepoint query` confirms cloud reparse tag `0x9000101a`
  - `dir` hangs as expected (FETCH_PLACEHOLDERS callback is stub)
  - Builds clean on both macOS and Windows
  - Verified on Windows 11 VM via SSH

---

## Phase 2: Directory Enumeration

- [x] **P2.1** FETCH_PLACEHOLDERS callback implementation
  - CfExecute(TRANSFER_PLACEHOLDERS) with DISABLE_ON_DEMAND_POPULATION flag
  - Single `block_on` call: resolve path, readdir, concurrent getattr (JoinSet)
  - FileIdentity = backslash-separated relative path from sync root
  - POPULATED_DIRS HashSet prevents duplicate placeholder creation
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P2.2** Root directory initial population
  - Handled by FETCH_PLACEHOLDERS callback on first access (no pre_populate)
  - pre_populate() kept as dead code for potential future use
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`, `distant-mount/src/lib.rs`

- [x] **P2.3** Nested directory traversal
  - `dir C:\CloudMount\src\` → 6 files, 4 dirs with real sizes
  - `dir C:\CloudMount\docs\` → 10 files, 3 dirs including file-provider/
  - Each subdirectory triggers exactly one FETCH_PLACEHOLDERS callback
  - Verified on Windows 11 VM

---

## Phase 3: File Hydration

- [x] **P3.1** FETCH_DATA callback implementation
  - Single block_on: resolve path + read in one async block
  - CfExecute(TRANSFER_DATA) with file content at requested offset
  - `type C:\CloudMount\rustfmt.toml` displays correct content
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [-] **P3.2** Chunked transfer for large files
  - Current: entire file transferred in single CfExecute (works for 144KB+)
  - Deferred: chunked transfer with progress for very large files (>4MB)
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [ ] **P3.3** CANCEL_FETCH_DATA callback
  - Record cancellation, discard pending read results
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

---

## Phase 4: Write Operations

- [x] **P4.1** NOTIFY_DELETE callback
  - Uses Utf8TypedPath for cross-platform path splitting (split_parent_name)
  - Single block_on: resolve parent + unlink/rmdir
  - CfExecute(ACK_DELETE) with STATUS_SUCCESS or STATUS_UNSUCCESSFUL
  - `del C:\CloudMount\PLAN.md` → file deleted on remote (verified)
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P4.2** NOTIFY_RENAME callback
  - Extracts TargetPath from callback params (Windows path)
  - Converts to relative path via relative_path() + normalize to /
  - CfExecute(ACK_RENAME) with status
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [x] **P4.3** New file creation (write-back)
  - ReadDirectoryChangesW watcher on dedicated OS thread
  - Overlapped I/O with 500ms polling for clean shutdown
  - Detects FILE_ACTION_ADDED, skips placeholders (REPARSE_POINT check)
  - Uploads via ChannelExt::write_file, converts via CfConvertToPlaceholder
  - MountGuard owns watcher lifecycle (stops before disconnect)
  - Verified: `echo hello > test.txt` on VM → file appears on remote Mac
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`,
    `distant-mount/src/lib.rs`, `distant-mount/Cargo.toml`

- [ ] **P4.4** File modification (write-back)
  - Detect modified hydrated files
  - Sync changes back to remote, mark in-sync
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

---

## Phase 5: Multiple Mounts & Status

- [x] **P5.1** Unique sync root IDs per mount
  - ID format: `distant!{hash(mount_path)}` using DefaultHasher
  - Each mount point gets a deterministic unique ID
  - Multiple daemon processes can mount different paths simultaneously
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`

- [-] **P5.2** Mount status detection
  - Cloud Files sync roots are registered at OS level but have no easy
    enumeration API (would need registry scanning under
    `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager`)
  - mount-status for cloud files deferred — `--foreground` mounts are
    visible via process list; daemon mounts have a separate issue
    (daemon processes exit immediately on Windows — pre-existing infra bug)
  - Files: `src/cli/commands/client.rs`

- [x] **P5.3** Selective unmount
  - `distant unmount C:\path` calls `unmount_path()` → CfUnregisterSyncRoot
  - Verified: "Unmounted C:\Users\senkwich\CloudMount"
  - Files: `distant-mount/src/backend/windows_cloud_files.rs`,
    `distant-mount/src/lib.rs`, `src/cli/commands/client.rs`

- [-] **P5.4** Unmount all
  - `distant unmount --all` calls `unmount()` (current process statics)
  - Works when run from the mount process; warns from a different process
  - Full fix needs sync root enumeration (not yet implemented)
  - Files: `src/cli/commands/client.rs`

---

## Test Infrastructure

- **Windows VM:** `ssh windows-vm` (passwordless)
- **Sync code:** `rsync -avz --exclude target/ --exclude .git/ /Users/senkwich/projects/distant/ windows-vm:/cygdrive/c/Users/senkwich/Projects/distant/`
- **Build:** `ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && cargo build"`
- **Run distant server:** `distant server listen` (on Mac or VM)
- **Connect:** `distant.exe connect distant://:<key>@<host>:<port>`
- **Mount:** `distant.exe mount --backend windows-cloud-files C:\Users\senkwich\CloudMount`
- **Verify:** `dir C:\Users\senkwich\CloudMount`
- **Logs:** Check stderr output (run with `DISTANT_LOG=trace`)

## Verification Commands (run on VM via SSH)

```bash
# Phase 1: Lifecycle
ssh windows-vm "dir C:\\Users\\senkwich\\CloudMount"
# Expect: empty directory with cloud overlay in Explorer

# Phase 2: Directory listing
ssh windows-vm "dir C:\\Users\\senkwich\\CloudMount"
# Expect: files from remote cwd

# Phase 3: File read
ssh windows-vm "type C:\\Users\\senkwich\\CloudMount\\somefile.txt"
# Expect: file contents

# Phase 4: Write operations
ssh windows-vm "echo hello > C:\\Users\\senkwich\\CloudMount\\test.txt"
# Verify on remote: file exists with "hello" content
ssh windows-vm "del C:\\Users\\senkwich\\CloudMount\\test.txt"
# Verify on remote: file is gone

# Phase 5: Multiple mounts
# Mount two different remote roots, verify both work independently
```
