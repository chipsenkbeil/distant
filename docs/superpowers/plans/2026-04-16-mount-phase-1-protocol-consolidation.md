# Phase 1: Protocol Consolidation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse 6 file I/O request variants into 2 with option structs, add wire-protocol error visibility (custom `Msg<T>` Deserialize + `hex_preview`), and update all backends.

**Architecture:** Extract Phase 1 changes from the existing `feature/file-mount` branch (worktree at `.claude/worktrees/file-mount/`) onto a fresh branch from master. The branch contains working, tested code — we are selectively extracting the protocol consolidation subset. Each task references the branch file as the source of truth for the final code.

**Tech Stack:** Rust, serde, rmp-serde, hex crate (already a dependency on master)

**Source branch reference:** `.claude/worktrees/file-mount/` — use `git diff master...feature/file-mount -- <path>` to see exact changes.

**Scope exclusions (belong to later phases):**
- `ApiServerHandler::from_arc` — Phase 3b (mount lifecycle)
- `is_session_closed` on SshApi — Phase 2 (network resilience)
- `sftp_io_error` helper changes to methods OTHER than `read_file`/`write_file` — out of scope
- `socket2` unconditional dependency — Phase 2
- `protocol/mount.rs`, `protocol/mod.rs` mount re-exports — Phase 3b

---

### Task 1: Create Branch + ReadFileOptions/WriteFileOptions

**Files:**
- Create: `distant-core/src/protocol/common/file_options.rs`
- Modify: `distant-core/src/protocol/common.rs` (or `common/mod.rs`)

- [ ] **Step 1: Create the feature branch**

```bash
git checkout master
git pull
git checkout -b feature/mount-phase-1
```

- [ ] **Step 2: Create `file_options.rs`**

Copy from `.claude/worktrees/file-mount/distant-core/src/protocol/common/file_options.rs`. The file defines `ReadFileOptions` (offset, len) and `WriteFileOptions` (offset, append) — both `Copy + Clone + Debug + Default + PartialEq + Eq + Serialize + Deserialize` with `#[serde(default, deny_unknown_fields)]`.

Note: `WriteFileOptions::append` uses `#[serde(skip_serializing_if = "utils::is_false")]` — `is_false` already exists in `distant-core/src/protocol/utils.rs` on master.

- [ ] **Step 3: Register the module**

In `distant-core/src/protocol/common.rs` (or `common/mod.rs`), add:
```rust
mod file_options;
```
in the module declarations, and:
```rust
pub use file_options::*;
```
in the re-exports. Place alphabetically alongside existing entries.

- [ ] **Step 4: Verify compilation**

```bash
cargo clippy --all-features -p distant-core
```

Expected: clean (new types are used by re-export, no dead code).

- [ ] **Step 5: Commit**

```bash
git add distant-core/src/protocol/common/file_options.rs distant-core/src/protocol/common.rs
git commit -m "feat(core): add ReadFileOptions and WriteFileOptions for protocol consolidation"
```

---

### Task 2: Consolidate Request Variants

**Files:**
- Modify: `distant-core/src/protocol/request.rs`

- [ ] **Step 1: Update imports**

Add `ReadFileOptions` and `WriteFileOptions` to the `use crate::protocol::common::{...}` import.

- [ ] **Step 2: Replace 6 enum variants with 2**

Remove `FileReadText`, `FileWriteText`, `FileAppend`, `FileAppendText`. Modify `FileRead` to add `#[serde(default)] options: ReadFileOptions`. Modify `FileWrite` to add `#[serde(default)] options: WriteFileOptions`. Use the branch file as reference for exact field ordering and doc comments.

- [ ] **Step 3: Update existing tests**

In the `file_read` test module: add `options: Default::default()` to all `Request::FileRead` constructions. Update JSON expectations to include `"options": {}`. Delete the entire `file_read_text` test module.

In the `file_write` test module: add `options: Default::default()` to all `Request::FileWrite` constructions. Update JSON expectations. Delete `file_write_text`, `file_append`, `file_append_text` test modules.

- [ ] **Step 4: Add new option-specific tests**

From the branch, add to the `file_read` module:
- `should_serialize_options_fields_when_set` — `ReadFileOptions { offset: Some(100), len: Some(256) }`
- `should_deserialize_options_fields_when_set`
- `should_be_able_to_roundtrip_options_via_msgpack`

Add to the `file_write` module:
- `should_serialize_append_option_when_true` — `WriteFileOptions { offset: None, append: true }`
- `should_deserialize_append_option_when_true`
- `should_be_able_to_roundtrip_append_option_via_msgpack`

- [ ] **Step 5: Verify**

```bash
cargo test --all-features -p distant-core -- protocol::request
cargo clippy --all-features -p distant-core
```

- [ ] **Step 6: Commit**

```bash
git add distant-core/src/protocol/request.rs
git commit -m "feat(core): consolidate FileRead/FileWrite request variants with option structs"
```

---

### Task 3: Consolidate Api Trait + Dispatch

**Files:**
- Modify: `distant-core/src/api.rs`

- [ ] **Step 1: Update imports**

Add `ReadFileOptions` and `WriteFileOptions` to the protocol import.

- [ ] **Step 2: Replace 6 trait methods with 2**

Remove `read_file_text`, `write_file_text`, `append_file`, `append_file_text` from the `Api` trait. Update `read_file` signature to accept `options: ReadFileOptions`. Update `write_file` signature to accept `data: Vec<u8>, options: WriteFileOptions`. Keep the default `unsupported` implementations. Use the branch file for exact signatures.

- [ ] **Step 3: Update `handle_request` dispatch**

Replace 6 match arms with 2. `FileRead { path, options }` dispatches to `api.read_file(ctx, path, options)` → `Response::Blob`. `FileWrite { path, data, options }` dispatches to `api.write_file(ctx, path, data, options)` → `Response::Ok`.

- [ ] **Step 4: Update MockApi and tests**

In the test module, update `MockApi::read_file` and `write_file` signatures to include options. Delete tests for removed methods (`default_read_file_text_returns_unsupported`, etc.). Update all `Request::FileRead`/`FileWrite` constructions in tests to include `options: Default::default()`.

- [ ] **Step 5: Verify**

```bash
cargo test --all-features -p distant-core -- api
cargo clippy --all-features -p distant-core
```

Note: This will NOT compile workspace-wide yet — backends still implement the old signatures. That's expected; we fix them in Tasks 5-7.

- [ ] **Step 6: Commit**

```bash
git add distant-core/src/api.rs
git commit -m "feat(core): consolidate Api trait file methods from 6 to 2 with options"
```

---

### Task 4: Update Client Extension Trait

**Files:**
- Modify: `distant-core/src/client/ext.rs`

- [ ] **Step 1: Update imports**

Add `ReadFileOptions` and `WriteFileOptions`.

- [ ] **Step 2: Update trait signatures**

`read_file` gains `options: ReadFileOptions` parameter. `write_file` gains `options: WriteFileOptions` parameter. Keep `read_file_text`, `write_file_text`, `append_file`, `append_file_text` as convenience methods (unchanged signatures).

- [ ] **Step 3: Update implementations**

- `read_file` impl: send `Request::FileRead { path, options }`
- `read_file_text` impl: delegate to `self.read_file(path, Default::default())`, convert bytes to string via `String::from_utf8_lossy`
- `write_file` impl: send `Request::FileWrite { path, data, options }`
- `write_file_text` impl: send `Request::FileWrite` with `data.into().into_bytes()` and `options: Default::default()`
- `append_file` impl: send `Request::FileWrite` with `WriteFileOptions { append: true, ..Default::default() }`
- `append_file_text` impl: same pattern with `.into_bytes()`

Use the branch file for exact code.

- [ ] **Step 4: Update tests**

Update all test callsites. `read_file_text` tests should now expect `FileRead` request (not `FileReadText`) and `Blob` response (not `Text`). Delete `read_file_text_should_return_error_on_mismatched_response` if it exists.

- [ ] **Step 5: Verify**

```bash
cargo test --all-features -p distant-core -- client::ext
cargo clippy --all-features -p distant-core
```

- [ ] **Step 6: Commit**

```bash
git add distant-core/src/client/ext.rs
git commit -m "feat(core): update client extension trait for consolidated file operations"
```

---

### Task 5: Update Host Backend

**Files:**
- Modify: `distant-host/src/api.rs`

- [ ] **Step 1: Update imports**

Add `ReadFileOptions`, `WriteFileOptions`. Add `AsyncReadExt`, `AsyncSeekExt` to tokio imports.

- [ ] **Step 2: Implement `read_file` with options**

Replace the simple `tokio::fs::read` call with offset/len support. Fast path when both are `None`. When `offset` is set, seek first. When `len` is set, read exactly that many bytes. Use the branch file for exact implementation.

- [ ] **Step 3: Implement `write_file` with options**

Replace the simple `tokio::fs::write` call. When `append` is true, open with `.append(true)`. When `offset` is set, open with `.write(true).truncate(false)` and seek. Default: `tokio::fs::write`.

- [ ] **Step 4: Remove deleted methods**

Delete `read_file_text`, `write_file_text`, `append_file`, `append_file_text` implementations.

- [ ] **Step 5: Update tests**

Delete tests for removed methods. Rename `append_file_*` tests to `write_file_append_*` and update to use `WriteFileOptions`. Add `Default::default()` to all other callsites.

- [ ] **Step 6: Verify**

```bash
cargo test --all-features -p distant-host
cargo clippy --all-features -p distant-host
```

- [ ] **Step 7: Commit**

```bash
git add distant-host/src/api.rs
git commit -m "feat(host): implement consolidated file operations with offset/len/append"
```

---

### Task 6: Update SSH Backend

**Files:**
- Modify: `distant-ssh/src/api.rs`

- [ ] **Step 1: Update imports and add `sftp_io_error` helper**

Add `ReadFileOptions`, `WriteFileOptions`. Add `AsyncSeekExt` to tokio imports. Add the `sftp_io_error` helper function that maps russh-sftp `StatusCode` to `io::ErrorKind`. Only use `sftp_io_error` in the `read_file` and `write_file` methods — do NOT retrofit it into other methods (that's out of scope).

- [ ] **Step 2: Implement `read_file` with options**

Add seek support when `offset` is set. Use `.take(len)` when `len` is set. Use the branch file for the exact SFTP open flags and seek/take pattern.

- [ ] **Step 3: Implement `write_file` with options**

When `append`, open with `WRITE | CREATE | APPEND`. When `offset`, open with `WRITE | CREATE` and seek. Default: `sftp.create()`. Use branch file for exact flag usage.

- [ ] **Step 4: Remove deleted methods**

Delete `read_file_text`, `write_file_text`, `append_file`, `append_file_text`.

- [ ] **Step 5: Verify**

```bash
cargo clippy --all-features -p distant-ssh
```

Note: SSH integration tests require sshd and may be slow. `cargo clippy` is the primary gate here.

- [ ] **Step 6: Commit**

```bash
git add distant-ssh/src/api.rs
git commit -m "feat(ssh): implement consolidated file operations with SFTP offset/len/append"
```

---

### Task 7: Update Docker Backend + CLI Callsites + api_tests

**Files:**
- Modify: `distant-docker/src/api.rs`
- Modify: `distant-core/tests/api_tests.rs`
- Modify: `src/cli/commands/client/copy.rs`
- Modify: `src/cli/commands/client.rs` (if it has `write_file`/`read_file` calls)

- [ ] **Step 1: Update Docker `read_file`**

Add post-read slicing for offset/len. When both are `None`, return full data. Otherwise slice `data[start..end]`. Use the branch file for exact implementation.

- [ ] **Step 2: Update Docker `write_file`**

When `append`: use exec-based `cat >>` with shell_quote, falling back to tar-read + append + tar-write. When `offset`: tar-read, patch range in memory, tar-write. Default: `tar_write_file`. Use the branch file for exact implementation.

- [ ] **Step 3: Remove deleted methods from Docker**

Delete `read_file_text`, `write_file_text`, `append_file`, `append_file_text`.

- [ ] **Step 4: Update `api_tests.rs`**

Update `TestApi` impls to new signatures. Update all `client.read_file(...)` calls to add `Default::default()`. Update all `RequestPayload::FileRead` constructions.

- [ ] **Step 5: Update CLI callsites**

In `src/cli/commands/client/copy.rs`, add `Default::default()` to `read_file()` and `write_file()` calls. Check `src/cli/commands/client.rs` for any direct file operation calls.

- [ ] **Step 6: Full workspace verification**

```bash
cargo fmt --all
cargo clippy --all-features --workspace --all-targets
cargo test --all-features --workspace
```

This is the first point where the entire workspace should compile and all tests should pass.

- [ ] **Step 7: Commit**

```bash
git add distant-docker/src/api.rs distant-core/tests/api_tests.rs src/cli/commands/client/copy.rs src/cli/commands/client.rs
git commit -m "feat(docker,cli): complete protocol consolidation across all backends and callsites"
```

---

### Task 8: Custom Msg<T> Deserialize

**Files:**
- Modify: `distant-core/src/protocol/msg.rs`

- [ ] **Step 1: Replace derive with manual Deserialize**

Remove `Deserialize` from the `#[derive(...)]` on `Msg<T>`. Add imports: `std::fmt`, `std::marker::PhantomData`, `serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer}`, `serde::de::{MapAccess, SeqAccess, Visitor}`, `serde::Deserializer`.

Implement `Deserialize for Msg<T>` with a `MsgVisitor` that uses `deserialize_any`: `visit_seq` → `Msg::Batch`, `visit_map` → `Msg::Single`. Copy the exact implementation from the branch file.

- [ ] **Step 2: Update existing tests**

The `single::should_be_able_to_deserialize_from_json` test must change from deserializing a bare string (which is neither map nor seq) to deserializing a struct. Add a `TestPayload { id: u32, name: String }` to the test module. Update the msgpack deserialize test similarly.

- [ ] **Step 3: Add `failure_paths` test module**

Copy the entire `failure_paths` module from the branch. It defines a `Tagged` enum with `#[serde(deny_unknown_fields, tag = "type")]` and tests that inner errors propagate verbatim (not collapsed into "did not match any variant of untagged enum"). 8-9 tests total.

- [ ] **Step 4: Verify**

```bash
cargo test --all-features -p distant-core -- protocol::msg
cargo clippy --all-features -p distant-core
```

- [ ] **Step 5: Commit**

```bash
git add distant-core/src/protocol/msg.rs
git commit -m "feat(core): custom Deserialize for Msg<T> preserves inner errors"
```

---

### Task 9: hex_preview Utility + Improved Error Messages

**Files:**
- Modify: `distant-core/src/net/common/utils.rs`
- Modify: `distant-core/src/net/client/channel.rs`
- Modify: `distant-core/src/net/server/connection.rs`

- [ ] **Step 1: Add hex_preview to utils.rs**

Add `HEX_PREVIEW_BYTES` constant (64) and `hex_preview(bytes, max)` function. Both `pub(crate)`. Uses `hex::encode` (already a dependency). Copy from branch.

- [ ] **Step 2: Improve `deserialize_from_slice` error**

Change the error format from `"Deserialize failed: {x}"` to `"Failed to deserialize {type_name} from {len} bytes: {e}"` using `std::any::type_name::<T>()`.

- [ ] **Step 3: Add tests for hex_preview and deserialize_from_slice**

From the branch, add `mod hex_preview` (5 tests: shorter_than_max, truncate, exact_max, empty, lowercase_hex) and `mod deserialize_from_slice` (2 tests: error_includes_type_name, happy_path).

- [ ] **Step 4: Update client channel.rs**

Add `use crate::net::common::utils;`. In `map_to_typed_mailbox`, replace the `trace!` + `error!` pair with a single `error!` that includes byte count and `utils::hex_preview(&res.payload, utils::HEX_PREVIEW_BYTES)`.

- [ ] **Step 5: Update server connection.rs**

Add `use crate::net::common::utils;`. Replace both decode error log sites:
1. Typed request decode failure: single `error!` with byte count + hex_preview of `request.payload`
2. Raw frame parse failure: single `error!` with byte count + hex_preview of `frame.as_item()`

**Important:** Only change the error logging in these two specific locations. Do NOT touch heartbeat, shutdown, or any other code in connection.rs (those belong to Phase 2).

- [ ] **Step 6: Verify**

```bash
cargo test --all-features -p distant-core -- net::common::utils
cargo clippy --all-features --workspace
```

- [ ] **Step 7: Commit**

```bash
git add distant-core/src/net/common/utils.rs distant-core/src/net/client/channel.rs distant-core/src/net/server/connection.rs
git commit -m "feat(core): add hex_preview utility and improve wire-protocol error visibility"
```

---

### Task 10: Final Verification + PR

- [ ] **Step 1: Full workspace verification**

```bash
cargo fmt --all
cargo clippy --all-features --workspace --all-targets
cargo nextest run --all-features --workspace --all-targets
```

All must pass with zero warnings.

- [ ] **Step 2: Verify only expected files touched**

```bash
git diff master...HEAD --stat
```

Expected files (and only these):
- `distant-core/src/protocol/common/file_options.rs` (new)
- `distant-core/src/protocol/common.rs`
- `distant-core/src/protocol/request.rs`
- `distant-core/src/protocol/msg.rs`
- `distant-core/src/api.rs`
- `distant-core/src/client/ext.rs`
- `distant-core/src/net/common/utils.rs`
- `distant-core/src/net/client/channel.rs`
- `distant-core/src/net/server/connection.rs`
- `distant-core/tests/api_tests.rs`
- `distant-host/src/api.rs`
- `distant-ssh/src/api.rs`
- `distant-docker/src/api.rs`
- `src/cli/commands/client/copy.rs`
- `src/cli/commands/client.rs` (if applicable)

- [ ] **Step 3: Push and create PR**

```bash
git push -u origin feature/mount-phase-1
gh pr create --title "Phase 1: Protocol consolidation (FileRead/FileWrite options + wire error visibility)" --body "$(cat <<'EOF'
## Summary
- Collapse FileRead/FileReadText/FileWrite/FileWriteText/FileAppend/FileAppendText into FileRead + FileWrite with ReadFileOptions/WriteFileOptions
- Custom Msg<T> Deserialize that preserves inner errors instead of collapsing to "did not match any variant"
- hex_preview utility for binary-safe error logging of wire payloads
- All backends (host, ssh, docker) updated with offset/len/append support

## Part of
Phase 1 of the file-mount branch decomposition. See docs/superpowers/specs/2026-04-16-file-mount-decomposition-design.md

## Test plan
- [ ] All existing tests pass (protocol consolidation is backwards-compatible via `#[serde(default)]`)
- [ ] New option-specific serde round-trip tests (ReadFileOptions, WriteFileOptions)
- [ ] Msg<T> failure_paths tests verify inner error propagation
- [ ] hex_preview unit tests
- [ ] Full workspace clippy + nextest clean
EOF
)"
```
