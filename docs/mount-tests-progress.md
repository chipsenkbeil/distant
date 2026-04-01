# Mount CLI Integration Tests — Progress

> Auto-updated by the `/mount-test-loop` command.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 1: Infrastructure

- [x] **P1.1** Add `mount` module to `tests/cli/mod.rs`
  - Feature-gated with `#[cfg(any(feature = "mount-fuse", ...))]`
  - Files: `tests/cli/mod.rs`, `tests/cli/mount/mod.rs`, `tests/cli/mount/browse.rs`

- [x] **P1.2** Implement `MountProcess` helper struct
  - Spawn with `new_std_cmd(["mount"])` + --foreground + --backend
  - Threaded stdout reader waits for "Mounted" (30s timeout)
  - Drop: kill + wait + `distant unmount` + remove dir
  - Files: `tests/cli/mount/mod.rs`

- [x] **P1.3** Implement `available_backends()` helper
  - Returns `Vec<&'static str>` via compile-time `#[cfg]` pushes
  - Excludes macos-file-provider (needs .app bundle — Phase 6)
  - Also: `seed_test_data()`, `verify_remote_file/exists/not_exists()`
  - Files: `tests/cli/mount/mod.rs`

- [x] **P1.4** Add nextest config for mount tests
  - `mount-integration` group with `max-threads = 1`
  - Override routes `test(mount::)` to this group
  - Files: `.config/nextest.toml`

---

## Phase 2: Core Read Tests

- [x] **P2.1** `browse.rs` — MNT-01, MNT-02, MNT-03
  - All 3 tests passing (NFS backend on macOS)
  - Files: `tests/cli/mount/browse.rs`

- [x] **P2.2** `file_read.rs` — FRD-01, FRD-02, FRD-03
  - Small file, 100KB file, nonexistent file — all passing
  - Files: `tests/cli/mount/file_read.rs`

- [x] **P2.3** `subdirectory.rs` — SDT-01, SDT-02
  - Subdir listing + deeply nested read — both passing
  - Files: `tests/cli/mount/subdirectory.rs`

---

## Phase 3: Write Tests

- [x] **P3.1** `file_create.rs` — FCR-01, FCR-02 — passing
- [x] **P3.2** `file_delete.rs` — FDL-01, FDL-02 — passing
- [x] **P3.3** `file_rename.rs` — FRN-01, FRN-02 — passing (cross-dir graceful skip)
- [x] **P3.4** `file_modify.rs` — FMD-01, FMD-02 — passing
  - FMD-02 (append) exposed a bug in RemoteFs::flush() that overwrote
    entire files instead of flushing dirty ranges. Fixed in remote.rs.
- [x] **P3.5** `directory_ops.rs` — DOP-01, DOP-02, DOP-03 — passing

---

## Phase 4: Mount Management

- [x] **P4.1** `readonly.rs` — RDO-01, RDO-02, RDO-03 — passing
  - Fixed: NFS now passes `,ro` mount option; FUSE now passes MountOption::RO
  - All backends enforce --readonly at the OS mount level

- [x] **P4.2** `remote_root.rs` — RRT-01, RRT-02 — passing

- [x] **P4.3** `multi_mount.rs` — MML-01, MML-02, MML-03 — passing

- [x] **P4.4** `status.rs` — MST-01, MST-02, MST-03 — passing
  - Fixed: mount-status guarded with is_running_in_app_bundle() to
    prevent ObjC nil crash outside .app
  - MST-03 cleans up stale NFS/FUSE mounts before asserting

- [x] **P4.5** `unmount.rs` — UMT-01, UMT-02, UMT-03 — passing
  - Fixed: unmount --all guarded with is_running_in_app_bundle()
  - Uses raw Command for unmount/mount-status (no --unix-socket arg)

---

## Phase 5: Edge Cases

- [x] **P5.1** `edge_cases.rs` — EDG-01 through EDG-05 — passing
  - Auto-create dir, file-as-mountpoint, special chars, rapid r/w, cleanup verify
  - Files: `tests/cli/mount/edge_cases.rs`

- [-] **P5.2** Backend-specific tests (BKE-*)
  - Deferred to Phase 6 (requires platform-specific test infrastructure)
  - NFS mount table, FUSE mount type, WCF sync root, FP domains

- [x] **P5.3** `daemon.rs` — DMN-01 — passing
  - Spawns mount without --foreground, reads "Mounted at" from parent
  - Lists directory via std::fs::read_dir to confirm mount works
  - Kills daemon via pkill, cleans up via umount -f + wait_for_unmount
  - Files: `tests/cli/mount/daemon.rs`

---

## Phase 6: FileProvider In-Test .app Bundle (macOS only)

- [ ] **P6.1** Replace hardcoded `APP_GROUP_ID` with plist reading
  - Read `NSExtensionFileProviderDocumentGroup` from .appex's Info.plist
  - .appex reads from own main bundle; host app reads from embedded .appex
  - Falls back to hardcoded default outside any bundle
  - No feature flag needed — plist contents drive behavior
  - Files: `distant-mount/src/backend/macos_file_provider/utils.rs`

- [ ] **P6.2** `build_test_app_bundle()` fixture (all in Rust)
  - Creates .app bundle directory structure
  - Copies test binary to app + appex locations
  - Copies production plists, replaces group ID with `group.dev.distant.test`
  - Writes test entitlements inline (no sandbox, no app-groups)
  - Signs with `codesign -s -` (ad-hoc), registers with `pluginkit -a`
  - Skips rebuild if up-to-date (mtime check)
  - No shell script, no test resource files
  - Files: `tests/cli/mount/mod.rs`

- [ ] **P6.3** FileProvider test setup + cases
  - `set_bin_path()` to bundled binary
  - Symlink manager socket into test container
  - Tests: list files, mount-status, unmount by URL, cleanup
  - Files: `tests/cli/mount/file_provider.rs`

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `ManagerCtx`
- **Mount helper:** `MountProcess` in `tests/cli/mount/mod.rs`
- **Backend iteration:** `available_backends()` returns compiled-in backends
- **Seed data:** Created via `distant fs write` / `distant fs make-dir`
- **Verification:** `distant fs read` / `distant fs exists`
- **Run tests:** `cargo nextest run --all-features -p distant -E 'test(mount)'`
  (MUST use nextest — `cargo test` ignores max-threads=1 and causes stale mounts)
