# Windows Cloud Files Implementation Loop

Iteratively implement the Windows Cloud Files mount backend. Each iteration
completes one requirement from the PRD, tests it on the Windows VM, and
updates progress.

## Context Files

Read these at the start of every iteration:

1. `docs/file-provider/windows-cloud-files-PRD.md` — full requirements
2. `docs/file-provider/windows-cloud-files-progress.md` — current status
3. `PLAN.md` — mission statement, success criteria, ground work instructions
4. The source files listed in the progress item you're working on

## Iteration Protocol

### Step 1: Select Next Item

If `$ARGUMENTS` contains a specific item ID (e.g., `P2.1`), work on that item.
If `$ARGUMENTS` is `status`, just read progress and report without changes.
Otherwise, select the first `[ ]` item in phase order (P0 before P1, etc.).

Items marked `[-]` (partial) should be prioritized over `[ ]` items in the
same phase — finish what's started before starting new work.

**Critical dependency**: Each phase builds on the previous. Do not skip ahead.

### Step 2: Understand

Use **rust-explorer** to:
- Read the specific source files mentioned in the progress item
- Read relevant Cloud Filter API documentation (the PRD has the key details)
- Read the macOS FileProvider backend (`distant-mount/src/backend/macos_file_provider.rs`)
  as a reference for patterns (Runtime usage, callback dispatch, etc.)
- Find existing utilities in distant-mount that can be reused

### Step 3: Implement

Use **rust-coder** to make the changes. Follow these rules:
- One logical change per iteration — don't bundle multiple items
- All edits happen on the Mac laptop (this machine), NOT on the Windows VM
- Run `cargo fmt --all` after every change
- Run `cargo clippy --all-features --workspace --all-targets` — note that
  `#[cfg(windows)]` code won't be checked by clippy on macOS. Be extra
  careful with:
  - Needless borrows (`.args(&[...])` → `.args([...])`)
  - Forward slashes in `PathBuf::join()` — use chained `.join()` calls
  - Type annotations that differ between platforms
- Follow CLAUDE.md anti-patterns (import modules not functions, no separator
  comments, no numbered comments in code)
- Use `log::debug!` for diagnostic output in callbacks
- Use `log::info!` for significant state transitions (register, connect, etc.)
- Use `log::error!` for failures

### Step 4: Sync and Test on Windows VM

After implementing, sync to the VM and test:

```bash
# Sync code to VM
rsync -avz \
    --exclude target/ \
    --exclude .git/ \
    /Users/senkwich/projects/distant/ \
    windows-vm:/cygdrive/c/Users/senkwich/Projects/distant/

# Build on VM
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && cargo build 2>&1"

# If build fails, fix on Mac and re-sync until it compiles

# For Phase 1+: Start a distant server (if not already running)
# Option A: Run on Mac
# distant server listen
# Option B: Run on VM
# ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && target/debug/distant.exe server listen --daemon"

# Connect and mount (on VM)
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && \
    target/debug/distant.exe mount --backend windows-cloud-files \
    C:\\Users\\senkwich\\CloudMount 2>&1"

# Verify (on VM)
ssh windows-vm "dir C:\\Users\\senkwich\\CloudMount"

# Cleanup
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && \
    target/debug/distant.exe unmount --all 2>&1"
```

Run with `DISTANT_LOG=trace` for detailed callback logging:
```bash
ssh windows-vm "cd /cygdrive/c/Users/senkwich/Projects/distant && \
    DISTANT_LOG=trace target/debug/distant.exe mount --backend windows-cloud-files \
    --foreground C:\\Users\\senkwich\\CloudMount 2>&1"
```

### Step 5: Validate

Use **code-validator** to review changes. Max 3 rounds of fixes.

For Windows-specific code that macOS clippy can't check:
- Manually review for needless borrows in `.arg()`/`.args()` calls
- Verify `PathBuf` operations use chained `.join()` not forward slashes
- Check that all `unsafe` blocks have SAFETY comments
- Verify error handling returns proper NTSTATUS codes

### Step 6: Update Progress

Edit `docs/file-provider/windows-cloud-files-progress.md`:
- Mark `[x]` if fully complete and verified on VM
- Mark `[-]` if partially done with notes on what remains
- Add discovered sub-tasks or blockers as notes under the item

### Step 7: Report

Summarize what was done:
```
== Windows Cloud Files Loop Iteration ==
Item:    P1.1 — Sync root registration
Status:  [x] Complete
Changes: windows_cloud_files.rs
VM Test: Registration succeeds, Explorer shows cloud folder
Notes:   Used progressive hydration instead of full
Next:    P1.2 — Sync root connection with callback table
```

## Key Implementation Patterns

### Callback function signature

Cloud Filter callbacks are C-style function pointers:
```rust
unsafe extern "system" fn callback(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    // Extract connection key, transfer key, file identity from info
    // Dispatch to async handler via Runtime::spawn()
    // Call CfExecute() with the response
}
```

### Bridging sync callbacks to async

```rust
// The callback thread is a dedicated thread from cldflt.sys,
// so blocking it with Handle::block_on() is safe.
let handle = TOKIO_HANDLE.get().expect("tokio handle not initialized");
let result = handle.block_on(async {
    let fs = REMOTE_FS.get().expect("remote fs not initialized");
    fs.readdir(ino).await
});
```

### Building CF_PLACEHOLDER_CREATE_INFO

```rust
let file_identity = relative_path.as_bytes();
let placeholder = CF_PLACEHOLDER_CREATE_INFO {
    RelativeFileName: PCWSTR(wide_name.as_ptr()),
    FsMetadata: CF_FS_METADATA {
        FileSize: file_size as i64,
        BasicInfo: FILE_BASIC_INFO {
            FileAttributes: if is_dir {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            },
            ..Default::default()
        },
    },
    FileIdentity: file_identity.as_ptr() as *const _,
    FileIdentityLength: file_identity.len() as u32,
    Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
    ..Default::default()
};
```

### CfExecute for TRANSFER_DATA

```rust
let op_info = CF_OPERATION_INFO {
    StructSize: size_of::<CF_OPERATION_INFO>() as u32,
    Type: CF_OPERATION_TYPE_TRANSFER_DATA,
    ConnectionKey: connection_key,
    TransferKey: transfer_key,
    ..Default::default()
};
let params = CF_OPERATION_PARAMETERS {
    ParamSize: /* calculated */,
    Anonymous: CF_OPERATION_PARAMETERS_0 {
        TransferData: CF_OPERATION_PARAMETERS_0_6 {
            CompletionStatus: STATUS_SUCCESS,
            Buffer: data.as_ptr() as *const _,
            Offset: offset as i64,
            Length: data.len() as i64,
            Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
        },
    },
};
unsafe { CfExecute(&op_info, &params as *const _ as *mut _) }?;
```

## Rules

- **One item per iteration** — keep changes focused
- **Always update progress.md** — this is the source of truth
- **Phase order matters** — don't skip ahead
- **Never commit** — changes live on Mac, synced to VM via rsync
- **Test on VM** — macOS compilation is necessary but not sufficient
- **Build after every change** — sync + `cargo build` on VM catches Windows-only errors
- **Log everything** — callbacks run on OS-managed threads; `log::debug!` is the diagnostic tool
- **Watch for 60s timeout** — Cloud Filter callbacks time out after 60 seconds; any `CfExecute` call resets all timers
