# TODO

Tracks open issues, technical debt, and planned work for the distant project.
This document is intended to be read by both humans and AI to understand what
needs to be done. **When an item is resolved, remove it entirely** — don't mark
it as "RESOLVED" or leave it in place.

Each item is tagged with a category:

- **(Bug)** — Produces incorrect results, wrong behavior, or data corruption.
- **(Enhancement)** — New feature or capability request.
- **(Refactor)** — Code restructuring that improves maintainability or
  performance.
- **(Limitation)** — Missing or unsupported functionality that is known and
  intentionally deferred.
- **(Workaround)** — Correct behavior achieved through a non-ideal mechanism.
  Works today but should be replaced with a cleaner solution.
- **(Acknowledgement)** — Known inconsistency or rough edge that is not
  currently causing failures but could in the future.
- **(Investigation)** — Needs research before deciding on an approach.

---

## Technical Debt

These are internal shortcuts and known rough edges that should eventually be
addressed.

### TD-0: Singleton test server — sshd/Docker cleanup after nextest

**(Enhancement)** The singleton test infrastructure (`distant-test-harness/src/singleton.rs`)
shares manager+server across all tests via file-lock coordination. Servers
auto-exit via `--shutdown lonely=10`. However, sshd and Docker containers
stay alive after nextest finishes (cleaned up on next run startup).

Options to explore:
1. Lock-file-watching reaper process that kills sshd when no shared locks remain
2. Nextest teardown scripts (not yet implemented in nextest — tracking issue #978)
3. `#[dtor]` from `ctor` crate works for `cargo test` (single process) but
   not for nextest (process-per-test, serial mount tests mean dtor fires after
   every test)

For now this is acceptable — sshd is ~2MB RSS, Docker container is idle.

### TD-1: Windows service integration incomplete

**(Limitation)** `win_service.rs` has `#![allow(dead_code)]` — Windows service
integration may be incomplete/untested.

### TD-2: Windows CI SSH test flakiness

**(Acknowledgement)** Windows CI SSH tests have intermittent failures tied to
VM performance variance on `windows-latest`. Mitigated with nextest retries
(4x in the default profile) and generous timeouts. Root causes found and fixed:
system sshd service conflicts, SFTP timeout defaults, and aggressive test read
deadlines.

### TD-3: `spawn --current-dir` flaky under parallel load

**(Bug)** `should_support_current_dir` test intermittently gets empty
stdout when running under parallel load. Root cause: `ProcDone` can arrive in
a separate transport frame before `ProcStdout`, causing
`process_incoming_responses` (in `distant-core/src/client/process.rs`) to
return immediately and drop `stdout_tx` before the stdout data is received.

- **Crate:** `distant-core`
- **File:** `distant-core/src/client/process.rs` (`process_incoming_responses`)
- **Fix:** After receiving `ProcDone`, continue draining the mailbox briefly
  to collect any remaining `ProcStdout`/`ProcStderr` data before returning.

### TD-4: Windows SSH copy cyclic-copy edge case

**(Workaround)** `distant-ssh` Windows `copy` uses a cmd.exe conditional
(`if exist "src\*"`) to dispatch between `copy /Y` (files) and
`xcopy /E /I /Y` (directories). `xcopy /I` treats the destination as a
directory, which causes "Cannot perform a cyclic copy" when src and dst are
sibling files in the same directory.

- **Crate:** `distant-ssh`
- **File:** `distant-ssh/src/api.rs`

### TD-5: Docker image pull has no CLI-visible progress

**(Limitation)** Docker image pull has no CLI-visible progress — `info!` logs
require `--log-level info` and go to the log file. Need a progress callback
mechanism (e.g. `ManagerResponse::Progress`) for real-time spinner updates
during long plugin operations like image pulls.

- **Crate:** `distant-docker`

### TD-6: Terminal programs hang in shell/ssh modes

**(Bug)** Running `distant ssh` or `distant shell` causes programs like `nvim`
(neovim) to hang with only a cursor visible, or `ntop` (top on Windows) to
hang after the first display frame. Terminal infrastructure has been rebuilt
with `TerminalSanitizer`, `TerminalFramebuffer`, and `PredictMode`
(in `src/cli/commands/common/`), but full-screen applications using alternate
screen buffers or relying on specific terminal capabilities may still have
issues. Needs retesting with current code.

- **Crate:** `distant` (binary), `distant-ssh`

### TD-7: SSH config HostName not respected

**(Bug)** Performing `distant ssh windows-vm` fails to connect to
`ssh://windows-vm` with "failed to lookup address information: nodename nor
servname provided, or not known" even though regular `ssh windows-vm` works
via `~/.ssh/config` with `HostName` directive.

- **Crate:** `distant-ssh`
- **File:** `distant-ssh/src/lib.rs` (connect logic, host resolution ~line 682)
- **Context:** The SSH config parsing uses `ssh2-config-rs` and does resolve
  `HostName` from config at line ~682 (`ssh_config.host_name.as_deref()`).
  The issue may be that the config is not loaded or queried with the right
  host alias, or that the destination is parsed before config lookup happens.

### TD-8: Windows ConPTY nested PTY exit detection

**(Bug)** `should_exit_on_eof_signal` test hangs on Windows for both Host and
SSH backends. After `pty-interactive` exits via "exit" command, `distant shell`
does not exit within 60s. The nested ConPTY chain (test ConPTY → distant shell
→ server ConPTY → pty-interactive) doesn't reliably propagate process exit
back to the outer ConPTY. Test is currently `#[ignore]` on Windows.

- **Crate:** `distant` (binary)
- **Files:** `src/cli/commands/common/terminal.rs`, `distant-host/src/api/process/pty.rs`
- **Context:** ConPTY is known to not send EOF on stdout pipes after child
  exit. The server has a 5s drain timeout for this, so ProcDone should still
  arrive. The hang may be in `distant shell`'s crossterm event loop or tokio
  runtime shutdown blocking on a ConPTY resource.

### TD-9: Windows CI SSH exec channel output failures

**(Bug)** Several `distant-ssh` integration tests fail on `windows-latest` CI:
commands execute but produce empty stdout/stderr, or hang waiting for output.
Tests pass on real Windows machines. Likely Windows OpenSSH exec channel
behavior under CI VM constraints. Tests are skipped via
`#[cfg_attr(all(windows, ci), ignore)]`.

- **Crate:** `distant-ssh`
- **Files:** `distant-ssh/tests/ssh/client.rs`, `distant-ssh/tests/ssh/ssh.rs`

---

## Open Issues

### Issue #229: Distant client-server hangs when switching networks

- **Type:** Bug
- **URL:** https://github.com/chipsenkbeil/distant/issues/229

**Problem:** When the network changes (e.g. switching WiFi), the connected
client becomes unresponsive. The TCP connection hangs instead of failing.

**Codebase context:** The server has a 5-second heartbeat that sends empty
frames (`distant-core/src/net/server/connection.rs:557-564`). The client has
reconnection strategies (`distant-core/src/net/client/reconnect.rs`) with
`ExponentialBackoff`, `FibonacciBackoff`, `FixedInterval`, and `Fail`
(default). However, no TCP keepalive socket options (`SO_KEEPALIVE`,
`TCP_KEEPIDLE`) are set on the transport
(`distant-core/src/net/common/transport/tcp.rs`), so the OS may not detect
the dead connection promptly.

**Work needed:**
1. Set `SO_KEEPALIVE` and related TCP keepalive options on sockets via
   `socket2` crate
2. Add read timeout on client side so it detects stale connections
3. Change default reconnect strategy from `Fail` to a backoff strategy, or
   make it configurable via CLI
4. Ensure heartbeat failure triggers reconnection rather than silent hang

---

### Issue #225: Build interface to extend CLI

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/225

**Problem:** No programmatic interface for extending the CLI. Desired for:
1. Adding commands like `distant server job <...>` for forking search
   processes
2. Moving launch logic for SSH back into the SSH crate with a builder that
   mirrors the CLI

**Codebase context:** CLI uses Clap v4 derive macros with hardcoded enum
variants for commands (`src/options.rs`). Adding new operations requires
modifying the enum — not pluggable at runtime.

**Work needed:**
1. Design a CLI extension trait or builder API
2. Allow plugins to register additional subcommands
3. Create a programmatic `ServerBuilder` that mirrors CLI args for
   `distant server listen`

---

### Issue #211: Update API to make headers available for search `external` option

- **Type:** Refactor (Breaking)
- **URL:** https://github.com/chipsenkbeil/distant/issues/211

**Problem:** Want to allow specifying whether search should use an external
tool (e.g. ripgrep) vs internal implementation, via request headers.

**Codebase context:** Request/response types in
`distant-core/src/net/common/packet/` already have optional `header` fields.
The `Api` trait methods don't receive headers. Search options in
`distant-core/src/protocol/common/search.rs` (`SearchQueryOptions`) have
extensive filtering but no `external` option. Headers are available in the
packet layer but not threaded through to the API trait.

**Work needed:**
1. Thread request headers through to `Api` trait method signatures
2. Add `external` option (boolean or string) to `SearchQueryOptions`
3. Breaking change: `Api` trait methods would need a new `headers` parameter
4. Alternative: use a context/middleware pattern to avoid changing every
   method signature

---

### Issue #198: Compress request & response byte formats

- **Type:** Refactor (Breaking)
- **URL:** https://github.com/chipsenkbeil/distant/issues/198

**Problem:** Protocol uses msgpack maps with string keys ("id", "payload",
"origin_id", "header") which wastes bytes. Switching to arrays (positional)
would save 9-24 bytes per message.

**Codebase context:** Manual msgpack encoding in
`distant-core/src/net/common/packet/request.rs` (lines ~183-205) and
`response.rs` (lines ~204-230) using `rmp::encode::write_map_len()`. Custom
parsing in `from_slice()` reads keys in order. Tests verify byte-level
compliance.

**Work needed:**
1. Change `write_map_len()` to `write_array_len()` in request/response
   encoding
2. Update `from_slice()` parsing to read positional array elements
3. Update all tests with new byte expectations
4. Breaking protocol change — needs version negotiation or minimum version
   bump

---

### Issue #192: Provide API for connection status change notifications

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/192

**Problem:** Reconnection happens silently. Clients need notifications for
connection state changes (connected, reconnecting, disconnected).

**Codebase context:** `ConnectionState` enum and `ConnectionWatcher` already
exist in `distant-core/src/net/client/reconnect.rs` with states
`Reconnecting`, `Connected`, `Disconnected`. The infrastructure exists but
may not be exposed to consumers of the client API or the neovim plugin.

**Work needed:**
1. Expose `ConnectionWatcher` through the public client API
2. Add CLI output for connection state changes
3. Wire notifications to the neovim plugin via the API protocol

---

### Issue #177: Support optional checksum for reading & writing files

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/177

**Problem:** No way to verify file integrity during read/write. Want:
1. Include checksum when reading files to detect changes
2. Provide checksum when writing to prevent overwriting modified files
   (optimistic concurrency control)

**Codebase context:** `FileRead`/`FileWrite` request types in
`distant-core/src/protocol/request.rs` have no checksum fields. File
operations transmit raw bytes without integrity verification.

**Work needed:**
1. Add optional `checksum` field to `FileRead`/`FileWrite` requests and
   responses
2. Decide on checksum algorithm (sha256 is suggested)
3. Server computes checksum on read; validates checksum on write
4. Protocol change — backwards compatible if optional

---

### Issue #164: [Investigate] Switch directory retrieval and file reading to streams

- **Type:** Investigation (Breaking)
- **URL:** https://github.com/chipsenkbeil/distant/issues/164

**Problem:** Large files and directories are batched entirely before sending,
which can cause memory pressure. Streaming would allow incremental delivery.

**Codebase context:** `FileRead` returns a single `Blob` response.
`DirRead` returns a single `DirEntries` response. No streaming protocol
exists.

**Work needed:**
1. Research streaming protocol design (chunked responses with sequence IDs)
2. Evaluate impact on all backends (host, SSH, Docker)
3. Consider backwards compatibility or version negotiation
4. Decision should be made before 1.0

---

### Issue #155: Add password auth as alternative to static-key

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/155

**Problem:** Only static-key authentication is supported for the distant
server. Want to add username/password authentication using OS-level
credential verification.

**Codebase context:** A `pwcheck` crate was created
(https://crates.io/crates/pwcheck) that uses `su` on Unix and `LogonUserW`
on Windows. The author wants to create a `distant-auth` crate with auth
methods as features (static-key always available, password as opt-in).

**Work needed:**
1. Create `distant-auth` crate with pluggable authentication methods
2. Integrate `pwcheck` crate as a `password` feature
3. Add PAM support as another feature (needs OpenPAM bindings for macOS/BSD)
4. Wire new auth methods into the server startup and manager
5. Significant work remaining — `pwcheck` crate exists but integration is
   not done

---

### Issue #145: Mount feature — remaining work

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/145

**Status:** Mount lifecycle is owned by the manager (A7 Phases 1-4 complete).
Four backends: NFS, FUSE (`fuser`), macOS FileProvider, Windows Cloud Files.
CLI commands `distant mount` (sends to manager, exits), `distant unmount`
(by ID or interactive), `distant status --show mount`. All mount tests
passing for NFS and FUSE host backends.

**Completed:**
- FUSE+SSH EIO bug fixed (SFTP error mapping + flush lock + path normalize)
- Readonly enforcement on all backends (RemoteFs + FP fileSystemFlags)
- FileProvider in cross-backend template (singleton via installed app)
- `--read-ttl` exposed, cache TTLs configurable
- Manager-owned mount lifecycle (MountPlugin trait, async unmount)

**Remaining work:**
1. **(Limitation)** `setattr` not implemented — requires distant protocol
   changes to support `chmod`/`chown`/`utime` on remote files.
2. **(Limitation)** Symlinks and hard links not implemented — needs protocol
   support for `symlink`, `readlink`, `link`.
3. **(Limitation)** File locking (POSIX `flock`/`fcntl`) not implemented.
4. **(Limitation)** Extended attributes (`xattr`) not implemented.
5. **(Limitation)** Large file streaming — files are fully buffered in
   memory. Need chunked read/write for files > RAM.
6. **(Enhancement)** Health monitoring: periodic checks per mount, connection
   drop → "disconnected" → reconnect → resume (A7 Phase 5).
7. **(Enhancement)** Process count audit: verify ~5 distant processes during
   full test run (A7 Phase 6).
8. **(Enhancement)** Windows Cloud Files automated testing via SSH to
   windows-vm (currently manual only).

---

---

### Issue #106: Support GitHub Codespaces port forwarding (automatic)

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/106

**Problem:** GitHub Codespaces automatically forwards ports when
`localhost:port` is printed to stdout. This could be leveraged during launch.

**Codebase context:** May already work — needs testing in a Codespace
environment.

**Work needed:**
1. Test if distant server output already triggers Codespace port forwarding
2. If not, ensure launch prints `localhost:<port>` to trigger it
3. Document Codespace usage

---

### Issue #75: Google Drive API integration

- **Type:** Investigation
- **URL:** https://github.com/chipsenkbeil/distant/issues/75

**Problem:** Provide a `distant-google-drive` plugin that wraps Google Drive
API v3 for file operations. Shell operations would return "unsupported".

**Codebase context:** Plugin architecture supports this — a new crate
implementing the `Plugin` trait with `connect()` would work. Would handle
`docker://`-style URIs like `gdrive://`.

**Work needed:**
1. Create `distant-google-drive` workspace member
2. Implement `Plugin` trait for Google Drive
3. Use `google-drive` or `google-drive3` crate
4. Handle OAuth2 authentication flow
5. Map distant API operations to Drive API calls
6. Low priority — marked as "consideration"

---

### Issue #69: Add websocket support for server & wasm for client

- **Type:** Enhancement / Refactor
- **URL:** https://github.com/chipsenkbeil/distant/issues/69

**Problem:** Enable distant client to run in WebAssembly (browser) by adding
WebSocket transport support.

**Codebase context:** Transport layer in
`distant-core/src/net/common/transport/` currently supports TCP and Unix
sockets. Adding WebSocket would require a new transport type using
`tokio-tungstenite`. WASM would need `distant-core` to have non-WASM code
behind feature flags.

**Work needed:**
1. Add WebSocket transport via `tokio-tungstenite`
2. Split `distant-core` non-wasm code into optional features
3. Create wasm-bindgen client that uses WebSocket transport
4. Large scope — requires significant refactoring of core

---

### Issue #2: SSDP or similar to detect existing distant instances

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/2

**Problem:** Auto-discover running distant servers/managers on the network
using SSDP or similar protocol.

**Codebase context:** No discovery protocol exists. Author suggests offering:
1. `distant list servers` — show available servers (showing port)
2. `distant list managers` — show available managers (showing pipe/socket)

**Work needed:**
1. Implement SSDP server/client using `tokio-ssdp` or similar
2. Register distant servers for discovery on startup
3. Add `distant list` CLI commands
4. Consider security implications of network discovery
5. Low priority — backlog item since project inception
