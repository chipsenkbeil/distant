# Manual Test Plan: distant mount Backends

This document is a step-by-step manual test plan for validating `distant mount`,
`distant unmount`, and `distant mount-status` across all mount backends. Tests
are parameterized: run each test once per backend available on your platform.

Every test is independent and can be run in isolation provided the setup steps
in Section 3 have been completed.

## Platform & Backend Matrix

| Backend | CLI Value | Linux | macOS | Windows | Daemonizes | Needs Mount Point |
|---------|-----------|-------|-------|---------|------------|-------------------|
| NFS | `nfs` | Yes | Yes | No | Yes | Yes |
| FUSE | `fuse` | Yes | Yes | No | Yes | Yes |
| Windows Cloud Files | `windows-cloud-files` | No | No | Yes | Yes | Yes |
| macOS FileProvider | `macos-file-provider` | No | Yes | No (OS-managed) | No |

**Notes:**
- NFS and FUSE backends spawn a long-running daemon process. The mount dies
  when the process exits.
- Windows Cloud Files registers a sync root with the OS. The daemon process
  must remain running.
- macOS FileProvider registers a domain with macOS; the OS manages the
  extension process. No mount point directory is needed. Files appear under
  `~/Library/CloudStorage/`.

## Prerequisites & Setup

### Per-Backend Prerequisites

- **NFS**: No special requirements on most Linux/macOS installations. The
  `mount_nfs` command must be available (ships with macOS and most Linux
  distros).
- **FUSE**: On macOS, install macFUSE and pkgconf: `brew install macfuse pkgconf`.
  On Linux, install the `fuse` or `fuse3` package for your distribution.
- **Windows Cloud Files**: Windows 10 or later. The mount point must be on an
  NTFS volume.
- **macOS FileProvider**: Requires the `Distant.app` bundle
  (see `docs/FILE_PROVIDER.md`). The binary must be run from inside the app
  bundle (`/Applications/Distant.app/Contents/MacOS/distant`).

### Server and Connection Setup

Start a distant server on the same machine for simplicity, then connect from
a second terminal.

```bash
# Terminal 1: Start server
distant server listen --host 127.0.0.1
# Output: distant://<KEY>@127.0.0.1:<PORT>
```

```bash
# Terminal 2: Connect (paste the URI from above)
distant connect distant://<KEY>@127.0.0.1:<PORT>
```

On Windows, substitute `distant.exe` for `distant` in all commands.

### Variable Definitions

Set these variables before running any tests. Adjust paths to suit your
environment.

```bash
# Unix (bash/zsh)
export BACKEND=nfs          # or: fuse, windows-cloud-files, macos-file-provider
export MOUNT1=/tmp/distant-mount-test1
export MOUNT2=/tmp/distant-mount-test2
export REMOTE_ROOT=$(pwd)   # or any directory the server can access
```

```cmd
:: Windows (cmd.exe)
set BACKEND=windows-cloud-files
set MOUNT1=C:\Users\%USERNAME%\DistantMount1
set MOUNT2=C:\Users\%USERNAME%\DistantMount2
set REMOTE_ROOT=C:\Users\%USERNAME%\distant-test-root
```

### Seed Data Creation

Create all test fixtures via `distant fs` commands, not through a mount. This
ensures the remote data is known-good regardless of backend state.

```bash
distant fs make-dir --all $REMOTE_ROOT/test-data
distant fs make-dir $REMOTE_ROOT/test-data/subdir
distant fs make-dir $REMOTE_ROOT/test-data/subdir/deep
distant fs make-dir $REMOTE_ROOT/test-data/empty-dir
distant fs write $REMOTE_ROOT/test-data/hello.txt "hello world"
distant fs write $REMOTE_ROOT/test-data/subdir/nested.txt "nested content"
distant fs write $REMOTE_ROOT/test-data/subdir/deep/deeper.txt "deep content"
```

Create a large file (~100 KB) for transfer tests:

```bash
# Unix
dd if=/dev/urandom bs=1024 count=100 2>/dev/null | base64 > /tmp/large-file.txt
distant fs write $REMOTE_ROOT/test-data/large-file.txt "$(cat /tmp/large-file.txt)"
```

```cmd
:: Windows (PowerShell equivalent)
$bytes = New-Object byte[] 102400; (New-Object Random).NextBytes($bytes);
[Convert]::ToBase64String($bytes) | Out-File C:\Temp\large-file.txt
distant.exe fs write %REMOTE_ROOT%\test-data\large-file.txt (Get-Content C:\Temp\large-file.txt -Raw)
```

## Backend-Parameterized Tests

For every test below, substitute `$BACKEND`, `$MOUNT1`, `$MOUNT2`, and
`$REMOTE_ROOT` with the values defined in setup. On Windows use `%VAR%`
syntax.

> **FileProvider note:** The macOS FileProvider backend ignores `$MOUNT1`.
> Do not pass a mount point argument. After mounting, browse files via Finder
> or at `~/Library/CloudStorage/Distant*/`. Where tests reference `$MOUNT1`,
> substitute the CloudStorage path for FileProvider.

---

### MNT-01: Mount and List Root Directory

**Setup:** No active mount.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
# Unix
ls $MOUNT1
```

```cmd
:: Windows
dir %MOUNT1%
```

**Expected:** Output lists `hello.txt`, `large-file.txt`, `subdir`, `empty-dir`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### MNT-02: Mount with --foreground

**Setup:** No active mount. Run this in a dedicated terminal.

```bash
distant mount --backend $BACKEND --foreground --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps (in a second terminal):**

```bash
ls $MOUNT1
```

**Expected:** Files are listed. The first terminal shows `Mounted at <path>` and
blocks until interrupted.

**Cleanup:** Press Ctrl+C in the foreground terminal. Verify the mount point is
no longer accessible:

```bash
ls $MOUNT1  # should fail or show empty directory
```

---

### MNT-03: Mount Default Remote Root

**Setup:** No active mount.

```bash
distant mount --backend $BACKEND $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1
```

**Expected:** Output lists the contents of the server's working directory
(the default remote root).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FRD-01: Read Small File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
# Unix
cat $MOUNT1/hello.txt
```

```cmd
:: Windows
type %MOUNT1%\hello.txt
```

**Expected:** Output is `hello world`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FRD-02: Read Large File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
# Unix
wc -c < $MOUNT1/large-file.txt
```

```cmd
:: Windows
(Get-Item %MOUNT1%\large-file.txt).Length
```

**Expected:** File size is approximately 137 KB (base64 of 100 KB). The read
completes without error or truncation.

**Remote Verification:**

```bash
distant fs read $REMOTE_ROOT/test-data/large-file.txt | wc -c
```

Both sizes should match.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FRD-03: Read Nonexistent File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
cat $MOUNT1/does-not-exist.txt
```

**Expected:** Error such as `No such file or directory` (exit code non-zero).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### SDT-01: List Subdirectory

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1/subdir
```

**Expected:** Output lists `nested.txt` and `deep`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### SDT-02: Read Deeply Nested File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
cat $MOUNT1/subdir/deep/deeper.txt
```

**Expected:** Output is `deep content`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FCR-01: Create New File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
# Unix
echo "created via mount" > $MOUNT1/created.txt
cat $MOUNT1/created.txt
```

```cmd
:: Windows
echo created via mount > %MOUNT1%\created.txt
type %MOUNT1%\created.txt
```

**Expected:** File is readable through the mount with content
`created via mount`.

**Remote Verification:**

```bash
distant fs read $REMOTE_ROOT/test-data/created.txt
```

Output matches `created via mount`.

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/created.txt
distant unmount $MOUNT1
```

---

### FCR-02: Create File in Subdirectory

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
echo "sub-created" > $MOUNT1/subdir/sub-created.txt
cat $MOUNT1/subdir/sub-created.txt
```

**Expected:** Content is `sub-created`.

**Remote Verification:**

```bash
distant fs read $REMOTE_ROOT/test-data/subdir/sub-created.txt
```

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/subdir/sub-created.txt
distant unmount $MOUNT1
```

---

### FDL-01: Delete File via Mount

**Setup:** Mount and create a disposable file.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs write $REMOTE_ROOT/test-data/to-delete.txt "delete me"
```

**Steps:**

```bash
# Unix
rm $MOUNT1/to-delete.txt
```

```cmd
:: Windows
del %MOUNT1%\to-delete.txt
```

**Expected:** File is removed. `ls $MOUNT1` no longer shows `to-delete.txt`.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/to-delete.txt
```

Output: `false`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FDL-02: Delete Nonexistent File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
rm $MOUNT1/ghost-file.txt
```

**Expected:** Error indicating the file does not exist (exit code non-zero).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### FRN-01: Rename File via Mount

**Setup:** Mount and create a disposable file.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs write $REMOTE_ROOT/test-data/old-name.txt "rename me"
```

**Steps:**

```bash
# Unix
mv $MOUNT1/old-name.txt $MOUNT1/new-name.txt
cat $MOUNT1/new-name.txt
```

```cmd
:: Windows
move %MOUNT1%\old-name.txt %MOUNT1%\new-name.txt
type %MOUNT1%\new-name.txt
```

**Expected:** Content is `rename me`. `old-name.txt` no longer exists.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/old-name.txt   # false
distant fs read $REMOTE_ROOT/test-data/new-name.txt     # "rename me"
```

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/new-name.txt
distant unmount $MOUNT1
```

---

### FRN-02: Rename File Across Subdirectories

**Setup:** Mount and create a disposable file.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs write $REMOTE_ROOT/test-data/move-me.txt "moving"
```

**Steps:**

```bash
mv $MOUNT1/move-me.txt $MOUNT1/subdir/moved.txt
cat $MOUNT1/subdir/moved.txt
```

**Expected:** Content is `moving`. Original path no longer exists.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/move-me.txt      # false
distant fs read $REMOTE_ROOT/test-data/subdir/moved.txt   # "moving"
```

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/subdir/moved.txt
distant unmount $MOUNT1
```

---

### FMD-01: Overwrite File Content

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs write $REMOTE_ROOT/test-data/overwrite-me.txt "original"
```

**Steps:**

```bash
echo "replaced" > $MOUNT1/overwrite-me.txt
cat $MOUNT1/overwrite-me.txt
```

**Expected:** Content is `replaced`.

**Remote Verification:**

```bash
distant fs read $REMOTE_ROOT/test-data/overwrite-me.txt
```

Output: `replaced`.

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/overwrite-me.txt
distant unmount $MOUNT1
```

---

### FMD-02: Append to File

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs write $REMOTE_ROOT/test-data/appendable.txt "line1"
```

**Steps:**

```bash
# Unix
echo "line2" >> $MOUNT1/appendable.txt
cat $MOUNT1/appendable.txt
```

```cmd
:: Windows
echo line2 >> %MOUNT1%\appendable.txt
type %MOUNT1%\appendable.txt
```

**Expected:** Content contains both `line1` and `line2`.

**Remote Verification:**

```bash
distant fs read $REMOTE_ROOT/test-data/appendable.txt
```

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/appendable.txt
distant unmount $MOUNT1
```

---

### DOP-01: Create Directory via Mount

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
# Unix
mkdir $MOUNT1/new-dir
ls -d $MOUNT1/new-dir
```

```cmd
:: Windows
mkdir %MOUNT1%\new-dir
dir %MOUNT1%\new-dir
```

**Expected:** Directory is created and listable.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/new-dir
```

Output: `true`.

**Cleanup:**

```bash
distant fs remove --force $REMOTE_ROOT/test-data/new-dir
distant unmount $MOUNT1
```

---

### DOP-02: Remove Empty Directory via Mount

**Setup:** Mount and create a disposable directory.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant fs make-dir $REMOTE_ROOT/test-data/removable-dir
```

**Steps:**

```bash
# Unix
rmdir $MOUNT1/removable-dir
```

```cmd
:: Windows
rmdir %MOUNT1%\removable-dir
```

**Expected:** Directory is removed.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/removable-dir
```

Output: `false`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### DOP-03: List Empty Directory

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1/empty-dir
```

**Expected:** No entries listed (empty output), exit code 0.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### RDO-01: Read-Only Mount Allows Reading

**Setup:** Mount as read-only.

```bash
distant mount --backend $BACKEND --readonly --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
cat $MOUNT1/hello.txt
```

**Expected:** Output is `hello world`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### RDO-02: Read-Only Mount Blocks File Creation

**Setup:** Mount as read-only.

```bash
distant mount --backend $BACKEND --readonly --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
echo "should fail" > $MOUNT1/blocked.txt
```

**Expected:** Error such as `Read-only file system` or `Permission denied`
(exit code non-zero). File is not created.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/blocked.txt
```

Output: `false`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### RDO-03: Read-Only Mount Blocks Deletion

**Setup:** Mount as read-only.

```bash
distant mount --backend $BACKEND --readonly --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
rm $MOUNT1/hello.txt
```

**Expected:** Error (exit code non-zero). File remains on the remote.

**Remote Verification:**

```bash
distant fs exists $REMOTE_ROOT/test-data/hello.txt
```

Output: `true`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### RRT-01: Custom Remote Root

**Setup:** No active mount.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data/subdir $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1
cat $MOUNT1/nested.txt
```

**Expected:** Root of the mount shows `nested.txt` and `deep`. Content of
`nested.txt` is `nested content`.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### RRT-02: Remote Root Points to Nonexistent Path

**Setup:** No active mount.

```bash
distant mount --backend $BACKEND --remote-root /nonexistent/path $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1
```

**Expected:** Either the mount command fails with an error, or `ls` returns an
error / empty listing. The behavior is backend-dependent; record what happens.

**Cleanup:**

```bash
distant unmount $MOUNT1 2>/dev/null || true
```

---

### MML-01: Two Simultaneous Mounts

**Setup:** No active mounts.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data/subdir $MOUNT2
```

**Steps:**

```bash
cat $MOUNT1/hello.txt
cat $MOUNT2/nested.txt
```

**Expected:** Both mounts are independently functional. `hello.txt` reads
`hello world`; `nested.txt` reads `nested content`.

**Cleanup:**

```bash
distant unmount $MOUNT1
distant unmount $MOUNT2
```

---

### MML-02: Unmount One of Two Mounts

**Setup:** Two active mounts from MML-01.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data/subdir $MOUNT2
```

**Steps:**

```bash
distant unmount $MOUNT1
cat $MOUNT2/nested.txt
```

**Expected:** `$MOUNT1` is no longer accessible. `$MOUNT2` still returns
`nested content`.

**Cleanup:**

```bash
distant unmount $MOUNT2
```

---

### MML-03: Mount Same Remote Root Twice

**Setup:** No active mounts.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT2
```

**Steps:**

```bash
cat $MOUNT1/hello.txt
cat $MOUNT2/hello.txt
```

**Expected:** Both mounts serve the same content independently. Alternatively,
the second mount may fail with a conflict error. Record the observed behavior.

**Cleanup:**

```bash
distant unmount $MOUNT1
distant unmount $MOUNT2 2>/dev/null || true
```

---

### MST-01: Mount Status Shows Active Mount

**Setup:** One active mount.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
distant mount-status
```

**Expected:** Output includes the mount point path (`$MOUNT1`) or the
FileProvider domain identifier.

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### MST-02: Mount Status JSON Format

**Setup:** One active mount.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
distant mount-status --format json
```

**Expected:** Valid JSON output containing the mount point. The JSON structure
varies by backend (NFS shows `"type": "nfs"`, Cloud Files shows
`"type": "cloud-files"`, FileProvider shows `"type": "file-provider"`).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### MST-03: Mount Status With No Mounts

**Setup:** No active mounts. Run `distant unmount --all` first.

```bash
distant unmount --all 2>/dev/null || true
```

**Steps:**

```bash
distant mount-status
```

**Expected:** Output is `No mounts found`.

---

### UMT-01: Unmount by Mount Point

**Setup:** One active mount.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
distant unmount $MOUNT1
ls $MOUNT1
```

**Expected:** Unmount succeeds with `Unmounted <path>` message. `ls` fails or
shows an empty directory (mount point may remain as an empty directory).

---

### UMT-02: Unmount All

**Setup:** Two active mounts.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data/subdir $MOUNT2
```

**Steps:**

```bash
distant unmount --all
distant mount-status
```

**Expected:** All mounts are removed. `mount-status` shows `No mounts found`.

---

### UMT-03: Unmount Nonexistent Mount Point

**Setup:** No active mount at `$MOUNT1`.

```bash
distant unmount --all 2>/dev/null || true
```

**Steps:**

```bash
distant unmount /tmp/not-a-real-mount
```

**Expected:** Error message (exit code non-zero). The exact message is
backend-dependent (e.g., `umount failed` on Unix).

---

### EDG-01: Mount Point Does Not Exist Yet

**Setup:** Ensure mount point directory does not exist.

```bash
rm -rf /tmp/distant-auto-create-test
```

**Steps:**

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data /tmp/distant-auto-create-test
ls /tmp/distant-auto-create-test
```

**Expected:** The mount command auto-creates the mount point directory. Files
are listed.

**Cleanup:**

```bash
distant unmount /tmp/distant-auto-create-test
```

---

### EDG-02: Mount Point Is a Regular File

**Setup:** Create a regular file where the mount point should be.

```bash
echo "blocker" > /tmp/distant-file-blocker
```

**Steps:**

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data /tmp/distant-file-blocker
```

**Expected:** Mount fails with an error (the path is a file, not a directory).

**Cleanup:**

```bash
rm /tmp/distant-file-blocker
```

---

### EDG-03: Special Characters in File Names

**Setup:** Create a file with spaces and special characters on the remote.

```bash
distant fs write "$REMOTE_ROOT/test-data/hello world.txt" "space content"
distant fs write "$REMOTE_ROOT/test-data/special!@#.txt" "special content"
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
cat "$MOUNT1/hello world.txt"
cat "$MOUNT1/special!@#.txt"
```

**Expected:** Both files are readable with correct content.

**Cleanup:**

```bash
distant fs remove "$REMOTE_ROOT/test-data/hello world.txt"
distant fs remove "$REMOTE_ROOT/test-data/special!@#.txt"
distant unmount $MOUNT1
```

---

### EDG-04: Rapid Sequential Read/Write

**Setup:** Mount at `$MOUNT1` with `--remote-root $REMOTE_ROOT/test-data`.

```bash
distant mount --backend $BACKEND --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
for i in $(seq 1 10); do
    echo "iteration $i" > $MOUNT1/rapid-test.txt
    content=$(cat $MOUNT1/rapid-test.txt)
    echo "Write $i -> Read: $content"
done
```

**Expected:** Each iteration reads back the value just written. No errors or
stale reads.

**Cleanup:**

```bash
distant fs remove $REMOTE_ROOT/test-data/rapid-test.txt
distant unmount $MOUNT1
```

---

### EDG-05: Server Disconnection During Mount

**Setup:** Mount at `$MOUNT1` with `--foreground` in a separate terminal.

```bash
# Terminal A
distant mount --backend $BACKEND --foreground --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:** In Terminal 1 (where the server is running), kill the server process
with Ctrl+C or `kill`.

```bash
# Terminal B
cat $MOUNT1/hello.txt
```

**Expected:** Reads fail with an I/O error or transport error after the server
is killed. The mount process should eventually exit or become stale. Record
observed behavior.

**Cleanup:**

```bash
distant unmount $MOUNT1 2>/dev/null || true
# Restart the server and reconnect for subsequent tests
```

---

## Backend-Specific Tests

These tests cover behavior unique to individual backends. Skip tests for
backends not available on your platform.

---

### BKE-01: NFS -- Verify OS Mount Table (Unix only)

**Setup:** Mount with NFS backend.

```bash
distant mount --backend nfs --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
mount | grep $MOUNT1
```

**Expected:** Output shows an NFS mount entry for `$MOUNT1` from `localhost`
(or `127.0.0.1`).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### BKE-02: NFS -- Custom Cache TTLs

**Setup:** Mount with custom TTL values.

```bash
distant mount --backend nfs --attr-ttl 10 --dir-ttl 10 \
    --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
ls $MOUNT1
cat $MOUNT1/hello.txt
```

**Expected:** Mount works normally. Files are readable. (TTL effects are
not directly observable in this test, but the flags must be accepted without
error.)

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### BKE-03: FUSE -- Verify FUSE Mount (Unix only)

**Setup:** Mount with FUSE backend.

```bash
distant mount --backend fuse --remote-root $REMOTE_ROOT/test-data $MOUNT1
```

**Steps:**

```bash
mount | grep $MOUNT1
```

**Expected:** Output shows a FUSE mount entry for `$MOUNT1` (typically with
filesystem type `macfuse` or `fuse`).

**Cleanup:**

```bash
distant unmount $MOUNT1
```

---

### BKE-04: Windows Cloud Files -- Verify Sync Root (Windows only)

**Setup:** Mount with Windows Cloud Files backend.

```bash
distant.exe mount --backend windows-cloud-files --remote-root %REMOTE_ROOT%\test-data %MOUNT1%
```

**Steps:**

```cmd
dir %MOUNT1%
distant.exe mount-status
```

**Expected:** Files are listed. `mount-status` shows a Cloud Files mount entry
with PID and mount point.

**Cleanup:**

```cmd
distant.exe unmount %MOUNT1%
```

---

### BKE-05: macOS FileProvider -- Mount Without Mount Point

**Setup:** No active FileProvider domains.

```bash
distant mount --backend macos-file-provider --remote-root $REMOTE_ROOT/test-data
```

**Steps:**

```bash
ls ~/Library/CloudStorage/
distant mount-status
```

**Expected:** A `Distant*` entry appears under `~/Library/CloudStorage/`.
`mount-status` shows a FileProvider domain with identifier and display name.

**Cleanup:**

```bash
distant unmount --all
```

---

### BKE-06: macOS FileProvider -- Mount Point Argument Rejected

**Setup:** No active mount.

**Steps:**

```bash
distant mount --backend macos-file-provider /tmp/should-fail
```

**Expected:** Error message: `macOS FileProvider does not accept a mount path`.
Exit code non-zero.

---

### BKE-07: macOS FileProvider -- Unmount by Destination URL

**Setup:** One active FileProvider domain connected to a known destination.

```bash
distant mount --backend macos-file-provider --remote-root $REMOTE_ROOT/test-data
```

**Steps:**

```bash
# Find the destination in mount-status output
distant mount-status
# Unmount using the destination URL
distant unmount ssh://user@host   # substitute actual destination
distant mount-status
```

**Expected:** The domain is removed. `mount-status` no longer lists it.

---

### BKE-08: Windows Cloud Files -- Unmount All

**Setup:** Two active Cloud Files mounts.

```cmd
distant.exe mount --backend windows-cloud-files --remote-root %REMOTE_ROOT%\test-data %MOUNT1%
distant.exe mount --backend windows-cloud-files --remote-root %REMOTE_ROOT%\test-data %MOUNT2%
```

**Steps:**

```cmd
distant.exe unmount --all
distant.exe mount-status
```

**Expected:** Both mounts are removed. `mount-status` shows no Cloud Files
entries.

---

## Summary Checklist

Record the result for each test per backend. Write **P** (pass), **F** (fail),
or **S** (skip) in each cell.

| Test ID | NFS | FUSE | WCF | FP | Notes |
|---------|-----|------|-----|-----|-------|
| MNT-01 | | | | | |
| MNT-02 | | | | | |
| MNT-03 | | | | | |
| FRD-01 | | | | | |
| FRD-02 | | | | | |
| FRD-03 | | | | | |
| SDT-01 | | | | | |
| SDT-02 | | | | | |
| FCR-01 | | | | | |
| FCR-02 | | | | | |
| FDL-01 | | | | | |
| FDL-02 | | | | | |
| FRN-01 | | | | | |
| FRN-02 | | | | | |
| FMD-01 | | | | | |
| FMD-02 | | | | | |
| DOP-01 | | | | | |
| DOP-02 | | | | | |
| DOP-03 | | | | | |
| RDO-01 | | | | | |
| RDO-02 | | | | | |
| RDO-03 | | | | | |
| RRT-01 | | | | | |
| RRT-02 | | | | | |
| MML-01 | | | | | |
| MML-02 | | | | | |
| MML-03 | | | | | |
| MST-01 | | | | | |
| MST-02 | | | | | |
| MST-03 | | | | | |
| UMT-01 | | | | | |
| UMT-02 | | | | | |
| UMT-03 | | | | | |
| EDG-01 | | | | | |
| EDG-02 | | | | | |
| EDG-03 | | | | | |
| EDG-04 | | | | | |
| EDG-05 | | | | | |
| BKE-01 | | S | S | S | NFS only |
| BKE-02 | | S | S | S | NFS only |
| BKE-03 | S | | S | S | FUSE only |
| BKE-04 | S | S | | S | WCF only |
| BKE-05 | S | S | S | | FP only |
| BKE-06 | S | S | S | | FP only |
| BKE-07 | S | S | S | | FP only |
| BKE-08 | S | S | | S | WCF only |

## Cleanup

After all tests are complete, perform a full teardown to leave the system
in a clean state.

```bash
# Unmount everything
distant unmount --all 2>/dev/null || true

# Kill any lingering distant mount daemon processes
pkill -f "distant mount" 2>/dev/null || true

# Remove seed data on the remote
distant fs remove --force $REMOTE_ROOT/test-data

# Remove local mount point directories
rm -rf $MOUNT1 $MOUNT2 /tmp/distant-auto-create-test /tmp/distant-file-blocker
```

On Windows:

```cmd
distant.exe unmount --all 2>nul
taskkill /F /IM distant.exe 2>nul
distant.exe fs remove --force %REMOTE_ROOT%\test-data
rmdir /S /Q %MOUNT1% 2>nul
rmdir /S /Q %MOUNT2% 2>nul
```
