# macOS File Provider — Implementation Progress

> Auto-updated by the `/file-provider-loop` command. Manual edits welcome.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 0: Diagnostics & Observability

- [x] **P0.1** Fix `logs-appex.sh` to check App Group container path
  - Script now checks App Group path first, falls back to legacy container path
  - Files: `scripts/logs-appex.sh`

- [x] **P0.2** Structured logging in all ObjC entry points
  - All entry points log at debug/trace level
  - Bootstrap success/failure at info/error level
  - Channel resolver logs connection attempts, fast-path hits, fallback searches, and outcomes
  - Files: `provider.rs`, `enumerator.rs`, `macos_file_provider.rs`, `macos_appex.rs`

- [x] **P0.3** `distant mount-status` subcommand
  - Lists registered FileProvider domains with metadata
  - Supports `--format shell` (table) and `--format json`
  - Exposes `DomainInfo` and `list_file_provider_domains()` from distant-mount
  - Files: `src/options.rs`, `src/cli/commands/client.rs`, `distant-mount/src/lib.rs`, `macos_file_provider.rs`

---

## Phase 1: Show Root Directory (Critical Path)

- [x] **P1.1** Handle working set container identifier
  - `enumerate_items` detects `NSFileProviderWorkingSetContainerItemIdentifier` and returns empty
  - `enumerate_changes` logs container for debugging
  - Files: `enumerator.rs`

- [x] **P1.2** Handle trash container identifier
  - `enumerate_items` detects `NSFileProviderTrashContainerItemIdentifier` and returns empty
  - Files: `enumerator.rs`

- [x] **P1.3** Handle root container in `itemForIdentifier`
  - Detects root constant, returns item with `NSFileProviderRootContainerItemIdentifier` as identifier
  - Files: `macos_file_provider.rs`

- [x] **P1.4** Map root container in enumerator
  - Root container → inode 1 mapping exists
  - Child items now use `NSFileProviderRootContainerItemIdentifier` as parent when ino=1
  - Files: `enumerator.rs`

- [x] **P1.5** Bootstrap succeeds and is visible
  - Bootstrap error stored in `BOOTSTRAP_ERROR` static
  - Enumerator signals error to Finder via `finishEnumeratingWithError` when bootstrap failed
  - Falls back to empty results when runtime not yet initialized (no error)
  - Files: `macos_file_provider.rs`, `enumerator.rs`

- [ ] **P1.6** Rebuild and verify root directory appears

---

## Phase 2: Browse Subdirectories & Open Files

- [x] **P2.1** `itemForIdentifier` for numeric inodes
  - Implementation exists with correct parent identifier resolution
  - Uses `resolve_parent_identifier` helper for consistent root constant handling
  - Files: `macos_file_provider.rs`

- [x] **P2.2** Fix parent identifier in enumerated items
  - Enumerator uses `NSFileProviderRootContainerItemIdentifier` when parent ino=1
  - Files: `enumerator.rs`

- [ ] **P2.3** Verify subdirectory navigation (manual test)

- [-] **P2.4** Verify file opening
  - `fetchContents` implementation exists (`macos_file_provider.rs:271-351`)
  - Reads entire file into memory (works for small files)
  - Untested end-to-end

---

## Phase 3: Write Operations

- [x] **P3.1** Fix `modifyItem` parent identifier
  - Uses `resolve_parent_identifier` helper instead of hardcoded `"1"`
  - Files: `macos_file_provider.rs`

- [x] **P3.2** `createItem` handles file content
  - Content URL read before spawning (NSURL not Send)
  - File content written to remote after creation
  - Uses `conformsToType(UTTypeFolder)` for directory detection
  - Re-fetches attr after writing so size/mtime are current
  - Files: `provider.rs`, `macos_file_provider.rs`

- [ ] **P3.3** Verify create operations (manual test)

- [ ] **P3.4** Verify modify operations (manual test)

- [ ] **P3.5** Verify delete operations (manual test)

---

## Phase 4: Robustness & Edge Cases

- [ ] **P4.1** Streamed file reads for large files
  - Current: `fs.read(ino, 0, u32::MAX)` loads entire file in RAM
  - Requires RemoteFs protocol changes for chunked reads — deferred

- [x] **P4.2** Per-domain Runtime (multi-mount)
  - `RwLock<HashMap<String, Arc<Runtime>>>` keyed by domain identifier
  - domain_id threaded through all handlers and enumerator
  - Per-domain bootstrap errors stored separately
  - Files: `macos_file_provider.rs`, `provider.rs`, `enumerator.rs`

- [x] **P4.3** Graceful bootstrap failure with proper error types
  - `make_fp_error` creates errors with `NSFileProviderErrorDomain`
  - Bootstrap failures use `NSFileProviderErrorCode::ServerUnreachable`
  - Files: `macos_file_provider.rs`, `enumerator.rs`

- [x] **P4.4** Better domain display names
  - Display name format: `Distant — ssh-root@host`
  - Files: `macos_file_provider.rs`

- [x] **P4.5** Filter `.` and `..` from readdir
  - Done at `enumerator.rs:111`

- [x] **P4.6** Handle symlinks
  - Uses getattr (resolves symlinks) for type determination
  - Symlinks to directories appear as folders in Finder
  - Files: `enumerator.rs`

---

## Phase 5: Change Notifications & Performance

- [ ] **P5.1** `signalEnumerator` on remote changes
- [ ] **P5.2** Meaningful `enumerateChanges`
- [x] **P5.3** Pagination for large directories
  - Pages of 100 items; page token is u64 offset as LE bytes
  - Initial page detected by non-8-byte length
  - Files: `enumerator.rs`
- [ ] **P5.4** Progress tracking for downloads/uploads
- [x] **P5.5** Cache warming on mount
  - Pre-enumerates root directory (inode 1) in background after bootstrap
  - First Finder open is instant; failure is non-fatal
  - Files: `macos_file_provider.rs`

---

## Phase 6: Polish & Production Readiness

- [ ] **P6.1** Reconnection on connection loss
- [x] **P6.2** Cleanup on unmount
  - Domain removal works
  - Metadata file cleanup works
  - Temp files (`/tmp/distant_fp_*`) cleaned up via `cleanup_temp_files()`
  - Files: `macos_file_provider.rs`
- [x] **P6.3** Multiple mount identification
  - Display name uses `Distant — ` prefix with sanitized destination
  - Each mount clearly identifiable in Finder sidebar
- [ ] **P6.4** Integration tests
- [ ] **P6.5** Documentation

---

## Infrastructure (already working)

- [x] Same binary serves CLI and `.appex` (detected via bundle path)
- [x] `define_class!` ObjC class registration before `NSExtensionMain`
- [x] App Group shared container for IPC (`39C6AGD73Z.group.dev.distant`)
- [x] Domain metadata serialized as JSON in `domains/<domain_id>`
- [x] Channel resolver connects to manager via shared Unix socket
- [x] `Runtime` async-to-sync bridge with lazy init via `watch::Receiver`
- [x] `RemoteFs` with 3-tier cache (attr, dir, read) and inode table
- [x] Domain registration/removal via `NSFileProviderManager`
- [x] Codesigning pipeline in `scripts/make-app.sh`
- [x] Entitlements: sandbox + app-groups + network-client on appex
- [x] Extension-Info.plist: fileprovider-nonui + enumeration support
- [x] File-based logging in appex process
- [x] `distant unmount --all` removes all domains
