# Mount Backend Fix Loop

Iteratively fix the NFS and FUSE mount backends. Each iteration:

1. Read `docs/file-provider/mount-backends-progress.md`
2. Pick the next incomplete item
3. Implement the fix
4. Run `cargo fmt --all && cargo clippy --all-features --workspace --all-targets`
5. Build and install: `scripts/make-app.sh`
6. Clean up existing mounts:
   ```bash
   pkill -f "distant m" || true
   sleep 1
   /Applications/Distant.app/Contents/MacOS/distant unmount --all 2>/dev/null || true
   ```
7. Test with `ssh://windows-vm` (passwordless):
   ```bash
   /Applications/Distant.app/Contents/MacOS/distant connect ssh://windows-vm
   /Applications/Distant.app/Contents/MacOS/distant mount --backend nfs --foreground ~/tmp/nfs-test
   # In another terminal: ls ~/tmp/nfs-test
   ```
8. Update `docs/file-provider/mount-backends-progress.md`
9. Commit with a descriptive message

## Important

- Use `/Applications/Distant.app/Contents/MacOS/distant` for ALL commands
- The `ssh://windows-vm` connection does NOT require a password
- Always kill existing distant processes before testing
- Test with `--foreground` first to see errors, then without
- The NFS backend requires `sudo` for `mount_nfs` on macOS
- Check `mount | grep nfs` to verify mounts
- Check `distant mount-status` to see registered mounts

## PRD Reference

See `docs/file-provider/mount-backends-PRD.md` for full requirements.
