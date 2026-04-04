# Mount Backends — Production Fixes & Full Test Coverage PRD

## Status (2026-04-04)

**234/234 mount tests passing.** All backends, zero skips. Major items:
- [x] 1. FUSE+SSH EIO — fixed (SFTP error mapping + flush lock + path normalization)
- [x] 2. FileProvider in template — done (singleton via installed app)
- [x] 3. Test shortcuts removed — mount_op_or_skip gone, catch_unwind replaced
- [x] 4. TTL CLI exposure — --read-ttl added
- [x] 5. Readonly — enforced at RemoteFs level for all backends
- [x] 6. TODO.md updated — deferred features documented
- [x] 7. Docker in test matrix — works, offset writes added
- [x] 8. All-green test matrix — 234/234 with zero skips
- [ ] 9. Windows VM script — not started
- [x] 10. Fixed sleeps replaced — polling helpers implemented

**A6 complete:** All 38 FP tests pass with zero skips. Fixes: readonly
fileSystemFlags + capabilities, delete/rename handlers, per-mount unmount,
remote root canonicalization, FP-specific test logic for rmdir/unmount/status.

**A7: Manager-owned mount lifecycle** (next major feature)

Architecture: Manager owns mount lifecycle via mount plugins. distant-core
uses generic types (Map/String) — no dependency on distant-mount. Mount
plugins register backends like connection plugins register schemes.

Key changes:
- Unified `distant status --show connections,mounts,tunnels` replaces
  `mount-status` (clean break for 0.21.0)
- `distant status --id <id>` works for any resource type
- `distant mount` sends Mount request to manager, returns immediately
- `distant unmount <id>` sends Unmount request (accepts multiple IDs)
- MountPlugin trait: NFS, FUSE, macOS FileProvider, Windows Cloud Files
- MountHandleOps trait: generic lifecycle (unmount, mount_point, etc.)
- Config flows as Map through protocol, parsed by each plugin
- Health monitoring: periodic checks per backend type
- Connection drop: mount → "disconnected" → reconnect → resume
- Manager shutdown: unmount all
- Process audit: expect ~5 distant processes (vs 30+ today)
- Windows testing via ssh windows-vm + rsync + cargo nextest

6 implementation phases — see progress.md for detailed checklist.

Additional completed work not in original requirements:
- Singleton test servers (Host, SSH, FileProvider)
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
