# Mount CLI Integration Tests — PRD

## Overview

Implement automated CLI integration tests for `distant mount`, `distant
unmount`, and `distant mount-status` that cover all test cases described in
`docs/MANUAL_TESTING.md`. Tests run against available mount backends (NFS,
FUSE, Windows Cloud Files, macOS FileProvider) parameterized at compile time
via `MountBackend::available_backends()`.

## Architecture

### File Organization

```
tests/
  cli_tests.rs          — existing; declares mod cli
  cli/
    mod.rs              — add mount module (feature-gated)
    mount/
      mod.rs            — declares submodules, mount test utilities
      browse.rs         — MNT-01..03: mount + dir listing
      file_read.rs      — FRD-01..03: read small, large, nested, missing
      subdirectory.rs   — SDT-01..02: traverse subdirs
      file_create.rs    — FCR-01..02: create file, verify on remote
      file_delete.rs    — FDL-01..02: delete file, verify gone
      file_rename.rs    — FRN-01..02: rename within and across dirs
      file_modify.rs    — FMD-01..02: overwrite and append
      directory_ops.rs  — DOP-01..03: mkdir, rmdir, list empty
      readonly.rs       — RDO-01..03: readonly mount blocks writes
      remote_root.rs    — RRT-01..02: scoped view, nonexistent root
      multi_mount.rs    — MML-01..03: two mounts, independent ops
      status.rs         — MST-01..03: mount-status shell + json
      unmount.rs        — UMT-01..03: by path, --all, nonexistent
      edge_cases.rs     — EDG-01..05: auto-create, special chars, etc.
```

Backend-specific tests (BKE-*) go in the same files where topically relevant,
gated by per-backend `#[cfg]` attributes.

### Test Infrastructure

**Context:** Use `ManagerCtx` (starts manager + server + auto-connects).
Mount tests need the server's remote root, which is the server's cwd.

**Mount helper struct:** A `MountProcess` that wraps a `Child` process running
`distant mount --foreground`. Provides:
- `spawn(ctx, backend, mount_point, extra_args)` — starts mount in foreground
- `wait_for_mounted()` — reads stdout for "Mounted" line
- `mount_point()` — returns the mount path
- Drop impl — kills the process and unmounts

This lives in `tests/cli/mount/mod.rs` alongside a `skip_if_backend_unavailable!`
macro.

**Backend parameterization:** Tests that work across ALL backends use a helper
function `available_backends() -> &'static [MountBackend]` that returns the list
of compiled-in backends. Each test iterates over this list. Tests
specific to one backend use `#[cfg(feature = "mount-nfs")]` etc.

**Seed data:** Created via `distant fs write` / `distant fs make-dir` through
the `ManagerCtx` before mounting. Verified via `distant fs read` /
`distant fs exists` after mount operations.

**Foreground mode only:** All tests use `--foreground` for controllability.
The daemon spawn path is a separate concern (already tested manually).

However, you should still create one final test that verifies mounting without
`--foreground` will still work and be available, but need to figure out how to
clean up the background process after. This should be its own standalone test.

### Feature Gating

In `tests/cli/mod.rs`:
```rust
#[cfg(any(
    feature = "mount-fuse",
    feature = "mount-nfs",
    feature = "mount-windows-cloud-files",
    feature = "mount-macos-file-provider",
))]
mod mount;
```

### Naming Convention (from TESTING.md)

- Flat test functions: `mount_<subject>_should_<behavior>`
- No nested test modules with `_tests` suffix
- No separator comments
- `#[rstest]` + `#[test_log::test]` for sync tests

### Test Case Mapping

Each MANUAL_TESTING.md test ID maps to one test function:

| ID | Function | File |
|----|----------|------|
| MNT-01 | `mount_should_list_root_directory` | browse.rs |
| MNT-02 | `mount_foreground_should_exit_on_kill` | browse.rs |
| MNT-03 | `mount_should_default_to_server_cwd` | browse.rs |
| FRD-01 | `read_should_return_file_contents` | file_read.rs |
| FRD-02 | `read_should_handle_large_file` | file_read.rs |
| FRD-03 | `read_should_fail_for_nonexistent_file` | file_read.rs |
| SDT-01 | `subdir_should_list_contents` | subdirectory.rs |
| SDT-02 | `deeply_nested_file_should_be_readable` | subdirectory.rs |
| FCR-01 | `create_file_should_appear_on_remote` | file_create.rs |
| FCR-02 | `create_file_in_subdir_should_appear_on_remote` | file_create.rs |
| FDL-01 | `delete_file_should_remove_from_remote` | file_delete.rs |
| FDL-02 | `delete_nonexistent_should_fail` | file_delete.rs |
| FRN-01 | `rename_file_should_update_remote` | file_rename.rs |
| FRN-02 | `rename_across_dirs_should_update_remote` | file_rename.rs |
| FMD-01 | `overwrite_file_should_sync_to_remote` | file_modify.rs |
| FMD-02 | `append_to_file_should_sync_to_remote` | file_modify.rs |
| DOP-01 | `mkdir_should_appear_on_remote` | directory_ops.rs |
| DOP-02 | `rmdir_should_remove_from_remote` | directory_ops.rs |
| DOP-03 | `empty_dir_should_list_nothing` | directory_ops.rs |
| RDO-01 | `readonly_mount_should_allow_reads` | readonly.rs |
| RDO-02 | `readonly_mount_should_block_writes` | readonly.rs |
| RDO-03 | `readonly_mount_should_block_deletes` | readonly.rs |
| RRT-01 | `remote_root_should_scope_listing` | remote_root.rs |
| RRT-02 | `nonexistent_remote_root_should_fail` | remote_root.rs |
| MML-01 | `two_mounts_should_show_independent_content` | multi_mount.rs |
| MML-02 | `unmount_one_should_not_affect_other` | multi_mount.rs |
| MML-03 | `same_root_twice_should_work_or_error` | multi_mount.rs |
| MST-01 | `mount_status_should_show_active_mount` | status.rs |
| MST-02 | `mount_status_json_should_be_valid` | status.rs |
| MST-03 | `mount_status_should_show_none_when_empty` | status.rs |
| UMT-01 | `unmount_by_path_should_succeed` | unmount.rs |
| UMT-02 | `unmount_all_should_remove_everything` | unmount.rs |
| UMT-03 | `unmount_nonexistent_should_fail` | unmount.rs |
| EDG-01 | `mount_should_auto_create_directory` | edge_cases.rs |
| EDG-02 | `mount_file_as_mountpoint_should_fail` | edge_cases.rs |
| EDG-03 | `special_chars_in_filename_should_work` | edge_cases.rs |
| EDG-04 | `rapid_read_write_should_not_corrupt` | edge_cases.rs |
| EDG-05 | `server_disconnect_should_error_gracefully` | edge_cases.rs |
| DMN-01 | `daemon_mount_should_list_directory` | daemon.rs |

Backend-specific tests (BKE-*) are included in the relevant files with
per-backend `#[cfg]` guards.

DMN-01 is a standalone test that verifies the daemon (non-`--foreground`)
mount path works. It spawns `distant mount` without `--foreground`, waits
for the "Mounted at" output, lists the directory to confirm it works, then
kills the daemon process by PID (extracted from the child spawn) and cleans
up via `distant unmount`.

## Phases

### Phase 1: Infrastructure (mod.rs + helpers)
- Feature-gated `mount` module in `tests/cli/mod.rs`
- `MountProcess` helper struct with spawn/wait/cleanup
- `available_backends()` helper
- `skip_if_backend_unavailable!` macro
- Nextest config for mount test group

### Phase 2: Core Read Tests (MNT, FRD, SDT)
- browse.rs, file_read.rs, subdirectory.rs
- 8 tests covering mount, listing, file reads, traversal

### Phase 3: Write Tests (FCR, FDL, FRN, FMD, DOP)
- file_create.rs, file_delete.rs, file_rename.rs, file_modify.rs, directory_ops.rs
- 11 tests covering all write operations

### Phase 4: Mount Management (RDO, RRT, MML, MST, UMT)
- readonly.rs, remote_root.rs, multi_mount.rs, status.rs, unmount.rs
- 14 tests covering mount configuration and lifecycle

### Phase 5: Edge Cases + Daemon (EDG, BKE, DMN)
- edge_cases.rs + backend-specific `#[cfg]` blocks
- daemon.rs — single daemon-mode smoke test
- 14 tests covering error paths, platform behavior, and daemon mode

### Phase 6: FileProvider In-Test .app Bundle (macOS only)

Build `target/test-Distant.app` during test setup so the FileProvider backend
can be included in `available_backends()` and tested via `ManagerCtx`.

- **P6.1** Add `testing` feature to `distant-mount`, `mount-testing` to workspace
- **P6.2** Gate `app_group_container_path()` override behind `#[cfg(feature = "testing")]`
  - File-based override at `/tmp/distant-test-container-override`
  - Cross-process safe (test writes, .appex reads)
  - Code absent from production builds (no feature = no code)
- **P6.3** Create test-specific entitlements (no sandbox, no app-groups)
  - `resources/macos/test-distant.entitlements`
  - `resources/macos/test-distant-appex.entitlements`
- **P6.4** `build_test_app_bundle()` fixture
  - Runs `scripts/build-macos-bundle.sh` with ad-hoc signing + test entitlements
  - Registers .appex via `pluginkit -a`
  - Creates temp container dir, writes override file
  - Symlinks manager socket into container
- **P6.5** Override `set_bin_path()` to use bundled binary
  - `target/test-Distant.app/Contents/MacOS/distant`
  - Makes `is_running_in_app_bundle()` return true
  - FileProvider becomes the default backend
- **P6.6** FileProvider-specific test cases
  - Listing files via `~/Library/CloudStorage/`
  - mount-status shows FileProvider domain
  - Unmount by destination URL
  - Domain cleanup on test teardown

## Phase 6 Implementation Details

### Why this is needed

The macOS FileProvider backend requires a `.app` bundle with a `.appex`
extension registered via PlugInKit. `fileproviderd` (an Apple system daemon)
launches the `.appex` as a separate process when Finder accesses the mount.
No open-source project has automated this testing — distant would be the first.

### Feature flag setup

**`distant-mount/Cargo.toml`:**
```toml
[features]
testing = []
```

**Workspace `Cargo.toml`:**
```toml
mount-testing = ["dep:distant-mount", "distant-mount/testing"]
```

`--all-features` (standard test command) enables both. Release builds with
explicit features never include `mount-testing`.

### app_group_container_path() override

**File:** `distant-mount/src/backend/macos_file_provider/utils.rs`

The override is gated by `#[cfg(feature = "testing")]` so the code is
completely absent from production builds:

```rust
#[cfg(feature = "testing")]
const CONTAINER_OVERRIDE_PATH: &str = "/tmp/distant-test-container-override";

pub fn app_group_container_path() -> Option<PathBuf> {
    #[cfg(feature = "testing")]
    if let Ok(override_path) = std::fs::read_to_string(CONTAINER_OVERRIDE_PATH) {
        let path = PathBuf::from(override_path.trim());
        if path.is_dir() {
            return Some(path);
        }
    }

    // Production: NSFileManager API (requires provisioning profile)
    let group_id = NSString::from_str(APP_GROUP_ID);
    let manager = NSFileManager::defaultManager();
    let url = manager.containerURLForSecurityApplicationGroupIdentifier(&group_id)?;
    let path_ns = url.path()?;
    Some(PathBuf::from(path_ns.to_string()))
}
```

This works cross-process: the test writes the file, and the `.appex` (launched
by `fileproviderd`) reads it. `fileproviderd` doesn't propagate env vars, so
the file-based approach is necessary.

All 13 call sites in the codebase go through `app_group_container_path()` or
`domains_dir()`, so no other code changes are needed.

### Test-specific entitlements

**`resources/macos/test-distant.entitlements`:**
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "...">
<plist version="1.0">
<dict>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
```

**`resources/macos/test-distant-appex.entitlements`:** Same content.

Omits `com.apple.security.app-sandbox` and `com.apple.security.application-groups`
which are restricted entitlements requiring provisioning profiles. Without
sandbox, the `.appex` can access any filesystem path including the override
container directory.

### build_test_app_bundle() fixture

In `tests/cli/mount/mod.rs`:

```rust
#[cfg(all(feature = "mount-testing", target_os = "macos"))]
fn build_test_app_bundle() -> PathBuf {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let binary = workspace.join("target/debug/distant");
    let bundle = workspace.join("target/test-Distant.app");
    let main_in_bundle = bundle.join("Contents/MacOS/distant");

    // Skip rebuild if bundle is up-to-date
    if main_in_bundle.exists() {
        let bin_mtime = std::fs::metadata(&binary).unwrap().modified().unwrap();
        let bun_mtime = std::fs::metadata(&main_in_bundle).unwrap().modified().unwrap();
        if bun_mtime >= bin_mtime {
            return bundle;
        }
    }

    // Build bundle with ad-hoc signing, test entitlements, no profiles
    let status = std::process::Command::new("bash")
        .arg(workspace.join("scripts/build-macos-bundle.sh"))
        .arg(&binary)
        .env("CODESIGN_IDENTITY", "-")
        .env("ENTITLEMENTS", workspace.join("resources/macos/test-distant.entitlements"))
        .env("APPEX_ENTITLEMENTS", workspace.join("resources/macos/test-distant-appex.entitlements"))
        .env("APP_PROFILE", "")
        .env("APPEX_PROFILE", "")
        .status()
        .expect("build-macos-bundle.sh failed");
    assert!(status.success());

    // Register with PlugInKit
    let appex = bundle.join("Contents/PlugIns/DistantFileProvider.appex");
    let _ = std::process::Command::new("pluginkit")
        .args(["-a", appex.to_str().unwrap()])
        .status();
    let _ = std::process::Command::new("pluginkit")
        .args(["-e", "use", "-i", "dev.distant.file-provider"])
        .status();

    bundle
}
```

### FileProvider test setup

```rust
#[cfg(all(feature = "mount-testing", target_os = "macos"))]
fn setup_file_provider_test(ctx: &ManagerCtx) -> TempDir {
    let bundle = build_test_app_bundle();

    // Override bin_path to use bundled binary
    let _ = set_bin_path(bundle.join("Contents/MacOS/distant"));

    // Create temp container and write override file
    let container = TempDir::new().unwrap();
    std::fs::create_dir_all(container.path().join("domains")).unwrap();
    std::fs::write(
        "/tmp/distant-test-container-override",
        container.path().to_str().unwrap(),
    ).unwrap();

    // Symlink the manager socket into the container
    std::os::unix::fs::symlink(
        &ctx.socket_or_pipe,
        container.path().join("distant.sock"),
    ).unwrap();

    container
}
```

### Cleanup

```rust
fn cleanup_file_provider_test() {
    let _ = std::fs::remove_file("/tmp/distant-test-container-override");
    // Domains are removed by distant unmount --all
}
```

### Key insight: same binary, two roles

The `distant` binary serves both as CLI and as `.appex`. At startup
(`src/main.rs`), if `NSBundle.bundlePath()` ends with `.appex`, it enters
`macos_appex::main()` (XPC listener, never returns). Otherwise it processes
CLI commands normally. The test uses the CLI path; `fileproviderd` launches
the `.appex` copy.

Both copies are the SAME binary (built with `--all-features` including
`testing`), so both have the container override code.

## Non-Goals

- Stress testing (covered by separate stress test infrastructure)
- Performance benchmarking
