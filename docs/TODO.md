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

### TD-1: Windows service integration incomplete

**(Limitation)** `win_service.rs` has `#![allow(dead_code)]` — Windows service
integration may be incomplete/untested.

### TD-2: Windows CI SSH test flakiness

**(Acknowledgement)** Windows CI SSH tests have intermittent failures tied to
VM performance variance on `windows-latest` — mitigated with nextest retries
(4x) and generous timeouts. Historical issues:

- **Consistent 19/47 failures**: System sshd service conflicting with per-test
  instances; resolved by stopping the system service in CI.
- **SFTP timeout**: `russh-sftp` defaults to a 10-second per-request timeout.
  Windows `sftp-server.exe` startup under CI load exceeded this. Resolved by
  using `SftpSession::new_opts()` with a unified SSH timeout constant (60s).
- **Test read deadlines**: proc_spawn tests had 5–15 second deadlines for
  reading stdout/stderr, too aggressive for slow Windows VMs. Increased to 30s.

### TD-3: Windows SSH copy cyclic-copy edge case

**(Workaround)** `distant-ssh` Windows `copy` uses a cmd.exe conditional
(`if exist "src\*"`) to dispatch between `copy /Y` (files) and
`xcopy /E /I /Y` (directories). `xcopy /I` treats the destination as a
directory, which causes "Cannot perform a cyclic copy" when src and dst are
sibling files in the same directory.

- **Crate:** `distant-ssh`
- **File:** `distant-ssh/src/utils.rs`

### TD-4: Docker image pull has no CLI-visible progress

**(Limitation)** Docker image pull has no CLI-visible progress — `info!` logs
require `--log-level info` and go to the log file. Need a progress callback
mechanism (e.g. `ManagerResponse::Progress`) for real-time spinner updates
during long plugin operations like image pulls.

- **Crate:** `distant-docker`

### TD-5: Terminal programs hang after termwiz removal

**(Bug)** Running `distant ssh` or `distant shell` after removing termwiz has
resulted in programs like `nvim` (neovim) hanging and not displaying anything
other than the cursor, or `ntop` (top on windows) hanging after the first
visual display of the processes (no refresh, no time tick displayed).

- **Crate:** `distant` (binary), `distant-ssh`
- **Context:** The PTY handling was changed when termwiz was removed. The
  current pty implementation may not correctly handle full-screen terminal
  applications that use alternate screen buffers or rely on specific terminal
  capabilities.

### TD-6: SSH config HostName not respected

**(Bug)** Performing `distant ssh windows-vm` fails to connect to
`ssh://windows-vm` with "failed to lookup address information: nodename nor
servname provided, or not known" even though regular `ssh windows-vm` works
via `~/.ssh/config` with `HostName` directive.

- **Crate:** `distant-ssh`
- **File:** `distant-ssh/src/lib.rs` (connect logic, host resolution)
- **Context:** The SSH config parsing uses `ssh2-config-rs` and does resolve
  `HostName` from config at line ~356 (`ssh_config.host_name.as_deref()`).
  The issue may be that the config is not loaded or queried with the right
  host alias, or that the destination is parsed before config lookup happens.
  This is also reported by external user in issue #251.
- **Related:** [#251](#issue-251), [#252](#issue-252)

---

## Open Issues

### Issue #252: Does not use keys from agent or from `identity_files` option

- **Type:** Bug
- **URL:** https://github.com/chipsenkbeil/distant/issues/252
- **Crate:** `distant-ssh`

**Problem:** SSH authentication fails with "unhandled auth case;
methods=PUBLIC_KEY, status={PUBLIC_KEY: Denied}" when using `identity_files`
option or when an ssh-agent is running. Users must explicitly set
`IdentityFile` in `~/.ssh/config` for keys to work.

**Codebase context:** The `distant-ssh` crate loads keys directly from files
only — there is no ssh-agent integration. Key loading logic is in
`distant-ssh/src/lib.rs` (lines ~675-738) with a three-tier resolution:
explicit CLI options → SSH config `IdentityFile` → default paths
(`~/.ssh/id_ed25519`, `id_rsa`, `id_ecdsa`). The `identity_files` CLI option
may not be correctly propagated or parsed (comma-separated paths from the
`--options` string in `plugin.rs` line ~196).

**Work needed:**
1. Fix `identity_files` option parsing to correctly load specified keys
2. Add ssh-agent support via `russh`'s agent client capabilities
3. Related to [#238](#issue-238)

---

### Issue #251: Fails to authenticate using public key if host is named in .ssh/config

- **Type:** Bug
- **URL:** https://github.com/chipsenkbeil/distant/issues/251

**Problem:** Using a Host alias (e.g. `ssh://nostromo`) from `~/.ssh/config`
fails with "Socket error: Connection reset by peer" or "unhandled auth case"
unless there is also a `Host` entry matching the raw IP address. Works only
after adding a `Host <IP>` entry alongside the named `Host` entry.

**Codebase context:** SSH config is parsed via `ssh2-config-rs`. The
`HostName` resolution at `distant-ssh/src/lib.rs:356` uses
`ssh_config.host_name.as_deref().unwrap_or(host.as_ref())`. The issue may be
that the config query doesn't match the alias correctly, or that the
`IdentityFile` from the config is not applied when connecting via alias.

**Work needed:**
1. Ensure SSH config is queried by the host alias (not the resolved hostname)
2. Verify all config directives (HostName, IdentityFile, User, Port) are
   applied when connecting via alias
3. Related to [#252](#issue-252), [TD-6](#td-6)

---

---

### Issue #238: Does not use ssh-agent to retrieve passwords for ssh-keys

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/238

**Problem:** `distant launch ssh://...` prompts for passphrase to decrypt SSH
key even when ssh-agent is running and has the key loaded. Regular `ssh` does
not prompt because it uses the agent.

**Codebase context:** The `distant-ssh` crate uses `russh` for SSH, which
does have agent client support (`russh-keys::agent`). However, the current
authentication flow in `lib.rs` only loads keys from files via
`decode_secret_key()` and never queries the SSH agent.

**Work needed:**
1. Add ssh-agent support using `russh-keys::agent::client`
2. Try agent authentication before falling back to key file loading
3. Handle agent forwarding if applicable
4. Related to [#252](#issue-252)

---

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

### Issue #186: Modernize release output

- **Type:** Enhancement
- **URL:** https://github.com/chipsenkbeil/distant/issues/186

**Problem:** Release binaries are larger than necessary and may require newer
glibc than target systems provide.

**Codebase context:** Release profile already uses `opt-level = "z"` and
`strip = true`. UPX compression was tried but causes issues on macOS.

**Work needed:**
1. Use release profile to strip (already done with `strip = true`)
2. Consider `panic = "abort"` to reduce binary size
3. Support nightly builds with `-Zbuild-std` for further optimization
4. Windows-specific size optimizations
5. Build Linux releases on older distros (via Docker) to reduce glibc
   version requirements
6. Consider static linking or musl target for Linux

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

### Issue #163: Support for Termux (Android)

- **Type:** Enhancement / Refactor
- **URL:** https://github.com/chipsenkbeil/distant/issues/163

**Problem:** Building on Termux (`aarch64-linux-android`) fails due to
`termios` crate incompatibility.

**Codebase context:** The `termios` issue was from the old `termwiz`
dependency which has since been removed. The nightly CI already builds for
`aarch64-linux-android` target. The `pty` feature in `distant-host` is
gated and can be disabled for Android. A Termux package was created upstream
(termux-packages PR #15610).

**Work needed:**
1. Verify current codebase compiles for `aarch64-linux-android` without
   `pty` feature (nightly CI already does this)
2. Ensure the Termux package stays up to date with releases
3. Document Termux installation and limitations (no PTY/shell support)
4. This may already be resolved — verify and close if so

---

### Issue #162: Cannot find known_hosts file if username has whitespace on Windows

- **Type:** Bug
- **URL:** https://github.com/chipsenkbeil/distant/issues/162

**Problem:** On Windows, if the username contains spaces (e.g. `C:\Users\fa
fa\.ssh\known_hosts`), distant cannot find the known_hosts file.

**Codebase context:** The old `wezterm-ssh` backend was the culprit. Current
`distant-ssh` uses `PathBuf` for known_hosts paths which handles spaces
correctly. Host key verification is now implemented (TOFU via russh's
`known_hosts` module). The path parsing in `plugin.rs` uses `PathBuf::from()`
which handles spaces fine.

**Work needed:**
1. Test known_hosts file paths with Windows usernames containing spaces
2. May be resolved by the backend switch — verify and close if so

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

### Issue #145: Support user-level file system mounting

- **Type:** Enhancement (Long-term)
- **URL:** https://github.com/chipsenkbeil/distant/issues/145

**Problem:** Want SSHFS-like mounting of remote filesystems.

**Codebase context:** No FUSE/filesystem mounting code exists in the
codebase. This is a large standalone feature.

**Work needed:**
1. Linux: Use `fuser` crate (modern Rust FUSE library)
2. macOS: Investigate Finder Sync Extension or macFUSE
3. Windows: Use Cloud Files API (Windows 10+)
4. Large feature — each platform has different requirements
5. All three platforms need the distant client API as the data source

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
