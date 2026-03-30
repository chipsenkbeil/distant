# Mount Backends — NFS & FUSE Fix-Up PRD

## Overview

The NFS and FUSE mount backends exist structurally but have critical bugs
preventing real-world use. This PRD covers fixes needed to make them work
alongside the macOS FileProvider backend.

## Current State

| Backend | Compiles | Works | Key Blocker |
|---------|----------|-------|-------------|
| macOS FileProvider | Yes | Yes | N/A — working |
| NFS | Yes | No | Requires root/sudo on macOS; no unmount cleanup |
| FUSE | Yes | Unknown | Requires macFUSE installed; untested |

## Architecture

All backends share `RemoteFs` (core/remote.rs) which translates filesystem
ops into distant protocol calls. The backends differ in how they bridge:

- **NFS**: Async-native via `nfsserve::vfs::NFSFileSystem` trait
- **FUSE**: Sync callbacks via `fuser::Filesystem`, bridged with `Runtime`
- **FileProvider**: ObjC callbacks, bridged with `Runtime`

## Phase 1: NFS Backend Fixes

### N1.1 — Fix root requirement on macOS

`mount_nfs` requires root. Either:
- Run `mount_nfs` via `sudo` (requires user to have sudo access)
- Or document that NFS mount requires `sudo distant mount --backend nfs`
- Or find a non-privileged mount approach

**Files**: `distant-mount/src/backend/nfs.rs:352-359`

### N1.2 — Add unmount cleanup

When the NFS server shuts down (shutdown signal or SIGTERM), call
`umount` / `diskutil unmount` on the mount point before exiting.

**Files**: `distant-mount/src/lib.rs:100-108`

### N1.3 — Capture child process errors

When daemonized, redirect child stderr to a log file instead of /dev/null
so mount errors can be diagnosed.

**Files**: `src/cli/common/spawner.rs`, `src/cli/commands/client.rs`

### N1.4 — Verify end-to-end NFS mount

Test with `--foreground` first, then daemonized:
```bash
sudo /Applications/Distant.app/Contents/MacOS/distant mount \
    --backend nfs --foreground ~/tmp/remote
```

### N1.5 — Fix readdir pagination

NFS readdir uses `skip_while(|e| e.ino <= start_after)` which assumes
inode ordering matches directory listing order. This may cause skipped
or duplicated entries.

**Files**: `distant-mount/src/backend/nfs.rs:220-260`

## Phase 2: FUSE Backend Fixes

### F2.1 — Verify FUSE backend compiles and runs

Requires macFUSE installed: `brew install macfuse pkgconf`
Test with:
```bash
/Applications/Distant.app/Contents/MacOS/distant mount \
    --backend fuse --foreground ~/tmp/fuse-remote
```

### F2.2 — Fix any FUSE-specific issues found during testing

### F2.3 — Add unmount cleanup for FUSE

FUSE has `AutoUnmount` configured but verify it works when the
daemon process is killed.

## Phase 3: Shared Infrastructure

### S3.1 — Log file for daemonized mount processes

Redirect stderr to a log file when daemonizing so errors are diagnosable:
`~/.local/share/distant/mount-{pid}.log` or similar.

### S3.2 — Improve `mount-status` for volume mounts

Show mount point, backend type, and PID for NFS/FUSE mounts.

### S3.3 — Improve error messages

When `mount_nfs` fails, suggest `sudo` or show the actual mount
command error output.

## Test Plan

For each backend, test with `ssh://windows-vm` (passwordless):
1. `connect ssh://windows-vm`
2. `mount --backend {nfs|fuse} [--foreground] ~/tmp/test`
3. `ls ~/tmp/test` — verify files appear
4. `cat ~/tmp/test/somefile` — verify file content
5. `mount-status` — verify mount is listed
6. `unmount ~/tmp/test` — verify clean unmount
7. Verify mount point directory is cleaned up

## Non-Goals

- Performance tuning (cache TTLs, prefetching)
- setattr implementation (chmod, truncate) — both backends are no-ops
- Symlink support in NFS/FUSE (returns ENOTSUPP)
