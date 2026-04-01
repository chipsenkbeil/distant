# Mount Test Implementation Loop

Iteratively implement mount CLI integration tests using the Backend x
MountBackend rstest template pattern. Each iteration completes one
progress item, writes the test code, verifies it compiles and passes,
and updates the progress tracker.

## Context Files

Read these at the start of every iteration:

1. `docs/mount-tests-PRD.md` — architecture, templates, file organization
2. `docs/mount-tests-progress.md` — current completion status
3. `docs/MANUAL_TESTING.md` — full test case descriptions
4. `docs/TESTING.md` — naming conventions

## Key Patterns

### Template usage (from PRD)

All mount tests use `#[apply(plugin_x_mount)]`:
```rust
use rstest_reuse::apply;
use distant_test_harness::mount::{plugin_x_mount, MountBackend, MountProcess};
use distant_test_harness::backend::Backend;
use distant_test_harness::skip_if_no_backend;

#[apply(plugin_x_mount)]
#[test_log::test]
fn mount_should_list_root_directory(
    #[case] backend: Backend,
    #[case] mount: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);
    let dir = ctx.unique_dir("mount-browse");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "hello.txt"), "hello world");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let mp = MountProcess::spawn(&ctx, mount, mount_dir.path(), &[
        "--remote-root", &dir,
    ]);

    let entries = std::fs::read_dir(mp.mount_point()).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert!(entries.contains(&"hello.txt".to_string()));
}
```

### Existing BackendCtx helpers

- `ctx.unique_dir("label")` — temp dir path valid for the backend
- `ctx.child_path(&dir, "name")` — join with correct separator
- `ctx.cli_write(&path, "content")` — create file via `distant fs write`
- `ctx.cli_read(&path) -> String` — read file via `distant fs read`
- `ctx.cli_exists(&path) -> bool` — check existence
- `ctx.cli_mkdir(&path)` — create directory
- `skip_if_no_backend!(backend)` — skip if unavailable

## Iteration Protocol

### Step 1: Select Next Item
First `[ ]` in phase order. `[-]` items take priority in same phase.
Phase 1 MUST complete before Phase 2.

### Step 2: Implement
- Use `#[apply(plugin_x_mount)]` for parameterized tests
- Use `BackendCtx` helpers for seed data and verification
- `MountProcess` for mount lifecycle
- `cargo fmt --all` + `cargo clippy --all-features --workspace --all-targets`

### Step 3: Test
```bash
cargo nextest run --all-features -p distant -E 'test(mount::)'
```

### Step 4: Update Progress
Mark `[x]` when all tests pass. Mark `[-]` with notes if partial.

### Step 5: Report
```
== Mount Test Loop Iteration ==
Item:    P2.1 — browse.rs (MNT-01..03)
Status:  [x] Complete
Tests:   host_nfs, host_fuse, ssh_nfs, ssh_fuse — all passing
Next:    P2.2 — file_read.rs
```

## Rules

- **One item per iteration**
- **Always update progress.md**
- **Phase order matters**
- **All tests must pass** before committing
- **Use nextest** (not `cargo test`)
- **Commit after each phase**
