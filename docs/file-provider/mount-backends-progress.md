# Mount Backends (NFS & FUSE) — Implementation Progress

> Auto-updated by the `/mount-backend-loop` command.
>
> Legend: `[x]` done, `[-]` partial/buggy, `[ ]` not started

---

## Phase 1: NFS Backend

- [ ] **N1.1** Fix root requirement on macOS
  - `mount_nfs` requires root/sudo
  - Files: `distant-mount/src/backend/nfs.rs:352-359`

- [ ] **N1.2** Add unmount cleanup on shutdown
  - Call `umount`/`diskutil unmount` when NFS server exits
  - Files: `distant-mount/src/lib.rs:100-108`

- [ ] **N1.3** Capture child process errors to log file
  - Redirect daemonized stderr to `~/.local/share/distant/mount-{pid}.log`
  - Files: `src/cli/common/spawner.rs`, `src/cli/commands/client.rs`

- [ ] **N1.4** Verify end-to-end NFS mount (manual test)
  - Test: `sudo distant mount --backend nfs --foreground ~/tmp/remote`
  - Verify: `ls ~/tmp/remote` shows remote files

- [ ] **N1.5** Fix readdir pagination ordering
  - Current: `skip_while(|e| e.ino <= start_after)` assumes inode order
  - Files: `distant-mount/src/backend/nfs.rs:220-260`

---

## Phase 2: FUSE Backend

- [ ] **F2.1** Verify FUSE compiles and runs
  - Requires macFUSE: `brew install macfuse pkgconf`
  - Test: `distant mount --backend fuse --foreground ~/tmp/fuse-remote`

- [ ] **F2.2** Fix FUSE-specific issues found during testing

- [ ] **F2.3** Verify FUSE unmount cleanup (AutoUnmount)

---

## Phase 3: Shared Infrastructure

- [x] **S3.1** Auto-create mount point directory
- [x] **S3.2** Daemonize mounts by default (`--foreground` opt-in)
- [x] **S3.3** SIGTERM handler for clean unmount
- [-] **S3.4** `mount-status` shows NFS/FUSE mounts
  - Detects localhost NFS mounts via `mount` command
  - Does not yet show PID or backend type

---

## Test Infrastructure

- Server: `ssh://windows-vm` (passwordless)
- Binary: `/Applications/Distant.app/Contents/MacOS/distant`
- Build: `scripts/make-app.sh`
- Cleanup: `pkill distant; distant unmount --all`
