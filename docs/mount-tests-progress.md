# Mount CLI Integration Tests — Progress

> Auto-updated by the `/mount-test-loop` command.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 1: Infrastructure

- [ ] **P1.1** Add `mount` module to `tests/cli/mod.rs`
  - Feature-gate with `#[cfg(any(feature = "mount-fuse", ...))]`
  - Files: `tests/cli/mod.rs`, `tests/cli/mount/mod.rs`

- [ ] **P1.2** Implement `MountProcess` helper struct
  - Spawn `distant mount --foreground --backend $BACKEND $MOUNT_POINT`
  - Wait for "Mounted" on stdout
  - Drop impl: kill process + unmount cleanup
  - Files: `tests/cli/mount/mod.rs`

- [ ] **P1.3** Implement `available_backends()` helper
  - Returns `Vec<&'static str>` of backend names available on this platform
  - Uses `#[cfg]` to build the list at compile time
  - Files: `tests/cli/mount/mod.rs`

- [ ] **P1.4** Add nextest config for mount tests
  - `mount-integration` test group with `max-threads = 1`
  - Prevents concurrent mount operations from interfering
  - Files: `.config/nextest.toml`

---

## Phase 2: Core Read Tests

- [ ] **P2.1** `browse.rs` — MNT-01, MNT-02, MNT-03
  - Mount and list root, foreground exit, default remote root
  - Files: `tests/cli/mount/browse.rs`

- [ ] **P2.2** `file_read.rs` — FRD-01, FRD-02, FRD-03
  - Read small file, large file, nonexistent file
  - Files: `tests/cli/mount/file_read.rs`

- [ ] **P2.3** `subdirectory.rs` — SDT-01, SDT-02
  - List subdir contents, read deeply nested file
  - Files: `tests/cli/mount/subdirectory.rs`

---

## Phase 3: Write Tests

- [ ] **P3.1** `file_create.rs` — FCR-01, FCR-02
  - Create file in root and subdir, verify on remote
  - Files: `tests/cli/mount/file_create.rs`

- [ ] **P3.2** `file_delete.rs` — FDL-01, FDL-02
  - Delete existing file, attempt delete nonexistent
  - Files: `tests/cli/mount/file_delete.rs`

- [ ] **P3.3** `file_rename.rs` — FRN-01, FRN-02
  - Rename within dir and across dirs
  - Files: `tests/cli/mount/file_rename.rs`

- [ ] **P3.4** `file_modify.rs` — FMD-01, FMD-02
  - Overwrite and append, verify sync to remote
  - Files: `tests/cli/mount/file_modify.rs`

- [ ] **P3.5** `directory_ops.rs` — DOP-01, DOP-02, DOP-03
  - mkdir, rmdir, list empty directory
  - Files: `tests/cli/mount/directory_ops.rs`

---

## Phase 4: Mount Management

- [ ] **P4.1** `readonly.rs` — RDO-01, RDO-02, RDO-03
  - Read-only mount allows reads, blocks writes and deletes
  - Files: `tests/cli/mount/readonly.rs`

- [ ] **P4.2** `remote_root.rs` — RRT-01, RRT-02
  - Custom remote root scopes listing, nonexistent root errors
  - Files: `tests/cli/mount/remote_root.rs`

- [ ] **P4.3** `multi_mount.rs` — MML-01, MML-02, MML-03
  - Two mounts with different roots, selective unmount
  - Files: `tests/cli/mount/multi_mount.rs`

- [ ] **P4.4** `status.rs` — MST-01, MST-02, MST-03
  - mount-status shell format, JSON format, empty
  - Files: `tests/cli/mount/status.rs`

- [ ] **P4.5** `unmount.rs` — UMT-01, UMT-02, UMT-03
  - Unmount by path, unmount --all, nonexistent path
  - Files: `tests/cli/mount/unmount.rs`

---

## Phase 5: Edge Cases

- [ ] **P5.1** `edge_cases.rs` — EDG-01 through EDG-05
  - Auto-create mount dir, file-as-mountpoint, special chars,
    rapid read/write, server disconnect
  - Files: `tests/cli/mount/edge_cases.rs`

- [ ] **P5.2** Backend-specific tests (BKE-*)
  - NFS mount table detection, FUSE mount type, WCF sync root,
    FP domain management — gated by per-backend `#[cfg]`
  - Files: distributed across relevant test files

- [ ] **P5.3** `daemon.rs` — DMN-01
  - Spawn `distant mount` WITHOUT `--foreground`, wait for "Mounted"
  - List directory via OS commands to confirm mount works
  - Kill daemon by PID, clean up via `distant unmount`
  - Standalone smoke test for daemon mount path
  - Files: `tests/cli/mount/daemon.rs`

---

## Phase 6: FileProvider In-Test .app Bundle (macOS only)

- [ ] **P6.1** Add `testing` feature to `distant-mount`, `mount-testing` to workspace
  - `distant-mount/Cargo.toml`: `testing = []`
  - Workspace `Cargo.toml`: `mount-testing = ["distant-mount/testing"]`
  - `--all-features` enables it; release builds exclude it
  - CLI tests use `#[cfg(feature = "mount-testing")]` to detect availability
  - Files: `distant-mount/Cargo.toml`, `Cargo.toml`

- [ ] **P6.2** Gate `app_group_container_path()` override
  - `#[cfg(feature = "testing")]` file-based override (crate-level feature)
  - Reads `/tmp/distant-test-container-override` for container path
  - Override code absent from production builds
  - Files: `distant-mount/src/backend/macos_file_provider/utils.rs`

- [ ] **P6.3** Create test-specific entitlements
  - No sandbox, no app-groups (allows ad-hoc signing)
  - Keep network.client + get-task-allow
  - Files: `resources/macos/test-distant.entitlements`,
    `resources/macos/test-distant-appex.entitlements`

- [ ] **P6.4** `build_test_app_bundle()` fixture
  - Run `build-macos-bundle.sh` with ad-hoc signing + test entitlements
  - Register .appex via `pluginkit -a`
  - Create temp container, write override file, symlink socket
  - Only build if not already up-to-date (check binary mtime)
  - Files: `tests/cli/mount/mod.rs`

- [ ] **P6.5** Override `set_bin_path()` for bundled binary
  - Point to `target/test-Distant.app/Contents/MacOS/distant`
  - `is_running_in_app_bundle()` returns true
  - FileProvider becomes default backend
  - Files: `tests/cli/mount/mod.rs`

- [ ] **P6.6** FileProvider test cases
  - List files via `~/Library/CloudStorage/` path
  - mount-status shows FileProvider domain
  - Unmount by destination URL
  - Domain cleanup on teardown
  - Files: `tests/cli/mount/file_provider.rs`

---

## Test Infrastructure

- **Harness:** `distant-test-harness` with `ManagerCtx`
- **Mount helper:** `MountProcess` in `tests/cli/mount/mod.rs`
- **Backend iteration:** `available_backends()` returns compiled-in backends
- **Seed data:** Created via `distant fs write` / `distant fs make-dir`
- **Verification:** `distant fs read` / `distant fs exists`
- **Build:** `cargo test --all-features -p distant -- mount`
- **Nextest:** `cargo nextest run --all-features -p distant -E 'test(mount)'`
