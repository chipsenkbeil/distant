---
description: >
  Iteratively implement the macOS File Provider. Reads progress.md, picks the
  next incomplete item, implements it, tests it, and updates progress. Designed
  for use with /ralph-loop for continuous development.
argument-hint: "[optional: specific item ID like P1.1, or 'status' to just report]"
---

# File Provider Implementation Loop

You are iteratively building the macOS File Provider for distant. Each
iteration completes one requirement from the PRD.

## Context Files

Read these at the start of every iteration:

1. `docs/file-provider/PRD.md` — full requirements and architecture
2. `docs/file-provider/progress.md` — current completion status
3. The source files listed in the progress item you're working on

## Iteration Protocol

### Step 1: Select Next Item

If `$ARGUMENTS` contains a specific item ID (e.g., `P1.1`), work on that item.
If `$ARGUMENTS` is `status`, just read `progress.md` and report current state
without making changes. Otherwise, select the first `[ ]` item in phase order
(P0 before P1 before P2, etc.). Skip items marked `[x]`.

Items marked `[-]` (partial) should be prioritized over `[ ]` items in the
same phase — finish what's started before starting new work.

**Critical dependency**: Phase 1 items (especially P1.1 and P1.2) MUST be
completed before anything in Phase 2+ can be verified. Do not skip ahead.

### Step 2: Understand

Use **rust-explorer** to:
- Read the specific files mentioned in the progress item
- Understand the current implementation state
- Identify what needs to change
- Find existing utilities in distant-mount that can be reused

### Step 3: Implement

Use **rust-coder** to make the changes. Follow these rules:
- One logical change per iteration — don't bundle multiple items
- Run `cargo fmt --all` and `cargo clippy --all-features --workspace --all-targets`
  after every change
- Follow CLAUDE.md anti-patterns (especially: import modules not functions,
  no separator comments, no numbered comments in code)
- Use `log::debug!` or `log::trace!` for diagnostic output in ObjC callbacks
- Use `log::info!` for significant state transitions (bootstrap, domain reg)
- Use `log::error!` for failures

### Step 4: Validate

Use **code-validator** to review the changes. Max 3 rounds of fixes.

For items that involve the `.appex` runtime behavior:
```bash
# Build and install
./scripts/make-app.sh

# Test cycle (requires an active SSH connection)
/Applications/Distant.app/Contents/MacOS/distant connect ssh://windows-vm
/Applications/Distant.app/Contents/MacOS/distant mount
# Observe Finder behavior
scripts/logs-appex.sh
/Applications/Distant.app/Contents/MacOS/distant unmount --all
```

Note: Manual Finder verification is required for most Phase 1+ items. Report
what you've confirmed via logs vs what needs manual Finder checking.

### Step 5: Update Progress

Edit `docs/file-provider/progress.md`:
- Mark the item `[x]` if fully complete
- Mark `[-]` if partially done with notes on what remains
- Add any discovered sub-tasks or blockers as notes under the item

### Step 6: Report

Summarize what was done:
```
== File Provider Loop Iteration ==
Item:    P1.1 — Handle working set container identifier
Status:  [x] Complete
Changes: provider.rs, enumerator.rs
Notes:   Working set returns empty enumerator. Verified via logs.
Next:    P1.2 — Handle trash container identifier
```

## Key Implementation Patterns

### Comparing NSString identifiers to framework constants

```rust
use objc2_file_provider::{
    NSFileProviderRootContainerItemIdentifier,
    NSFileProviderWorkingSetContainerItemIdentifier,
    NSFileProviderTrashContainerItemIdentifier,
};

// SAFETY: These are valid Apple framework constants
let root_id = unsafe { NSFileProviderRootContainerItemIdentifier };
let working_set_id = unsafe { NSFileProviderWorkingSetContainerItemIdentifier };
let trash_id = unsafe { NSFileProviderTrashContainerItemIdentifier };

if container_str == root_id.to_string() {
    // handle root
} else if container_str == working_set_id.to_string() {
    // handle working set
}
```

### Sending NSFileProviderError

```rust
use objc2_file_provider::NSFileProviderErrorDomain;

fn make_fp_error(code: isize, message: &str) -> Retained<NSError> {
    let domain = unsafe { NSFileProviderErrorDomain };
    let description = NSString::from_str(message);
    let key: &NSErrorUserInfoKey = unsafe { NSLocalizedDescriptionKey };
    let user_info = NSDictionary::from_retained_objects(
        &[key],
        &[Retained::into_super(Retained::into_super(description))],
    );
    unsafe { NSError::errorWithDomain_code_userInfo(domain, code, Some(&user_info)) }
}
```

### UnsafeSendable pattern for async blocks

ObjC completion handlers from block2 are `!Send`. Wrap them in
`UnsafeSendable` before moving into `rt.spawn()`:

```rust
let observer = macos_file_provider::UnsafeSendable(observer.retain());
rt.spawn(move |fs| async move {
    // use observer.method() — Deref makes this work through the wrapper
});
```

## Rules

- **One item per iteration** — keep changes focused and reviewable
- **Always update progress.md** — this is the source of truth
- **Phase order matters** — don't skip ahead to Phase 3 if Phase 1 isn't done
- **Build after every change** — `cargo clippy --all-features` catches errors
  that `cargo check` misses on macOS-only code
- **Log everything** — the extension runs in a separate process; printf
  debugging via `log::debug!` is the primary diagnostic tool
- **Test with make-app.sh** — compile-only verification is insufficient for
  FileProvider work; the extension must be rebuilt, signed, and installed
