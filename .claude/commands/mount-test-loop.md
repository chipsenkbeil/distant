# Mount Test Implementation Loop

Iteratively implement mount CLI integration tests. Each iteration completes
one progress item, writes the test code, verifies it compiles and passes,
and updates the progress tracker.

## Context Files

Read these at the start of every iteration:

1. `docs/mount-tests-PRD.md` — test architecture, file organization, naming
2. `docs/mount-tests-progress.md` — current completion status
3. `docs/MANUAL_TESTING.md` — full test case descriptions with expected behavior
4. `docs/TESTING.md` — naming conventions and test patterns
5. The source files listed in the progress item you're working on

## Iteration Protocol

### Step 1: Select Next Item

If `$ARGUMENTS` contains a specific item ID (e.g., `P2.1`), work on that item.
If `$ARGUMENTS` is `status`, just read progress and report without changes.
Otherwise, select the first `[ ]` item in phase order (P1 before P2, etc.).

Items marked `[-]` (partial) take priority over `[ ]` in the same phase.

**Critical dependency**: Phase 1 (infrastructure) MUST be completed before
any other phase. Do not skip ahead.

### Step 2: Understand

Use **rust-explorer** to:
- Read existing CLI test files for patterns (e.g., `tests/cli/client/fs_read_file.rs`)
- Read the `ManagerCtx` harness (`distant-test-harness/src/manager.rs`)
- Read the MANUAL_TESTING.md test cases you're implementing
- Find reusable utilities in the test harness

### Step 3: Implement

Use **rust-coder** to write the test code. Follow these rules:
- One progress item per iteration (one or two test files)
- Follow TESTING.md naming: `mount_<subject>_should_<behavior>`
- Use `#[rstest]` + `#[test_log::test]` for sync tests
- Use `ManagerCtx` for test context
- Seed data via `ctx.new_assert_cmd(["fs", "write"])` etc.
- Verify via `ctx.new_assert_cmd(["fs", "read"])` etc.
- Mount via `MountProcess::spawn()` helper
- Iterate over `available_backends()` for parameterized tests
- `cargo fmt --all` after every change
- `cargo clippy --all-features --workspace --all-targets` after every change

### Step 4: Validate

Run the tests:
```bash
cargo test --all-features -p distant -- mount
```

If tests fail:
- Read the error output carefully
- Fix the implementation
- Re-run until green

Use **code-validator** for code review. Max 3 rounds.

### Step 5: Update Progress

Edit `docs/mount-tests-progress.md`:
- Mark `[x]` if all tests in the item pass
- Mark `[-]` if partially done with notes on what remains
- Add any discovered issues as notes under the item

### Step 6: Report

Summarize what was done:
```
== Mount Test Loop Iteration ==
Item:    P2.1 — browse.rs (MNT-01, MNT-02, MNT-03)
Status:  [x] Complete
Tests:   3 tests, all passing
Changes: tests/cli/mount/browse.rs
Next:    P2.2 — file_read.rs
```

## Key Patterns

### MountProcess helper (Phase 1)

```rust
pub struct MountProcess {
    child: Child,
    mount_point: PathBuf,
}

impl MountProcess {
    pub fn spawn(
        ctx: &ManagerCtx,
        backend: &str,
        mount_point: &Path,
        remote_root: Option<&str>,
    ) -> Self {
        let mut cmd = ctx.new_std_cmd(["mount"]);
        cmd.args(["--backend", backend, "--foreground"]);
        if let Some(root) = remote_root {
            cmd.args(["--remote-root", root]);
        }
        cmd.arg(mount_point);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let child = cmd.spawn().expect("Failed to spawn mount");
        // ... wait for "Mounted" on stdout ...
        Self { child, mount_point: mount_point.to_path_buf() }
    }
}

impl Drop for MountProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // cleanup mount point
    }
}
```

### Backend iteration

```rust
fn available_backends() -> Vec<&'static str> {
    let mut backends = Vec::new();
    #[cfg(feature = "mount-nfs")]
    backends.push("nfs");
    #[cfg(all(feature = "mount-fuse", any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
    backends.push("fuse");
    #[cfg(all(feature = "mount-windows-cloud-files", target_os = "windows"))]
    backends.push("windows-cloud-files");
    // Skip macos-file-provider — requires .app bundle, not testable in CLI
    backends
}
```

### Seed data pattern

```rust
fn seed_test_data(ctx: &ManagerCtx, root: &Path) {
    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(root.join("subdir").to_str().unwrap())
        .assert()
        .success();
    ctx.new_assert_cmd(["fs", "write"])
        .args([root.join("hello.txt").to_str().unwrap(), "hello world"])
        .assert()
        .success();
}
```

### Verify on remote

```rust
fn verify_file_exists(ctx: &ManagerCtx, path: &Path) {
    ctx.new_assert_cmd(["fs", "exists"])
        .arg(path.to_str().unwrap())
        .assert()
        .success()
        .stdout("true\n");
}
```

## Rules

- **One item per iteration** — keep changes focused
- **Always update progress.md** — source of truth
- **Phase order matters** — infrastructure before tests
- **All tests must pass** — never commit failing tests
- **Follow TESTING.md** — naming, structure, no separators
- **Run `cargo fmt` + `cargo clippy`** before each commit
- **Commit after each phase** — not after each item
