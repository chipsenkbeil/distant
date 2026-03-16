# Manual Test Plan — `distant` CLI

## 1. Introduction

This document is a comprehensive, black-box manual test plan for the `distant`
CLI (version 0.21.0-dev, features: docker, host, pty, ssh). It treats every
command as an opaque binary — tests are derived entirely from `--help` output
and documented behavior, not from source code.

**Purpose:**

- Serve as the authoritative reference for what an automated E2E test framework
  must eventually cover.
- Provide step-by-step manual verification procedures a senior test engineer
  can follow on any supported platform.
- Catalog every external tool dependency so the Rust test harness knows exactly
  what cross-platform replacement binaries it must supply.

**Philosophy:**

- Every leaf command, flag, and option gets at least one test case.
- Every command gets at least one happy-path and one failure-mode test.
- Cross-platform differences are called out explicitly.
- Self-to-self (localhost) testing is the default — no second machine needed.

**CLI surface area:** 38 leaf commands, 6 global options, 5 predict modes,
2 output formats, 6 service-manager backends.

---

## 2. Test Environment Setup

All tests assume a self-to-self (localhost) configuration. The tester runs the
`distant` manager, server, and client commands on the same machine.

### 2.1 Prerequisites

| Requirement | Notes |
|-------------|-------|
| `distant` binary | In `$PATH` or referenced by absolute path |
| Local `sshd` | Running on port 22 (or custom port) accepting localhost connections |
| SSH key pair | `ssh-keygen -t ed25519 -f ~/.ssh/test_distant -N ""` — add public key to `~/.ssh/authorized_keys` |
| Docker runtime | Required only for Docker URI tests (`docker://...`). Can be Docker Desktop or Podman |
| `nc` (netcat) | For TCP tunnel verification |
| `jq` | For parsing JSON-format output |
| Writable temp directory | `$TMPDIR` or `/tmp` (Unix), `%TEMP%` (Windows) |

### 2.2 Starting the Test Environment

**Step 1 — Start the manager:**

```bash
distant manager listen --daemon
```

Verify it is running:

```bash
distant manager version
```

Expected: prints version/capabilities without error.

**Step 2 — Launch a server (self-to-self via SSH):**

```bash
distant launch ssh://localhost
```

Expected: prints connection ID. The active connection is now set.

**Step 3 — Verify the connection:**

```bash
distant status
```

Expected: shows one active connection to `ssh://localhost`.

### 2.3 Teardown

```bash
distant kill <CONNECTION_ID>
# Or if only one connection:
distant kill
# Then stop the manager (if running as daemon):
# Kill the manager process directly, or use service stop if installed
```

---

## 3. Cross-Platform Matrix

### 3.1 Supported Platforms

| Platform | Manager Socket | Service Manager | Path Separator | Shell |
|----------|---------------|-----------------|----------------|-------|
| Linux | Unix socket (`--unix-socket`) | systemd, openrc | `/` | `/bin/sh`, `/bin/bash` |
| macOS | Unix socket (`--unix-socket`) | launchd | `/` | `/bin/zsh`, `/bin/bash` |
| Windows | Named pipe (`--windows-pipe`) | sc, winsw | `\` | `cmd.exe`, `powershell.exe` |
| FreeBSD | Unix socket (`--unix-socket`) | rcd | `/` | `/bin/sh` |
| NetBSD | Unix socket (`--unix-socket`) | rcd | `/` | `/bin/sh` |
| OpenBSD | Unix socket (`--unix-socket`) | rcd | `/` | `/bin/sh` |

### 3.2 Platform-Specific Behavioral Differences

| Area | Unix (Linux/macOS/BSD) | Windows |
|------|----------------------|---------|
| Manager IPC | Unix domain socket | Named pipe |
| Socket option | `--unix-socket PATH` | `--windows-pipe NAME` |
| File permissions | POSIX mode bits (`chmod`) | readonly / notreadonly only |
| Symlinks | `ln -s` freely | Requires elevated privileges |
| PTY support | Native | ConPTY (may differ in behavior) |
| Path format | `/home/user/file` | `C:\Users\user\file` |
| Daemon mode | `--daemon` forks | `--daemon` not applicable; use service |
| Service install | launchd/systemd/openrc/rcd | sc/winsw |
| Default shell | `$SHELL` | `cmd.exe` |

---

## 4. Dependency Catalog

This section catalogs every external tool a human tester would reach for across
all test cases. Each entry describes what it is, why tests need it, and the
behavioral contract for a cross-platform Rust test harness equivalent.

### 4.1 Network Utilities

#### `nc` (netcat)

- **What:** TCP/UDP network utility for reading/writing data across connections.
- **Why:** Tunnel tests need a TCP listener to accept forwarded connections and
  a TCP sender to push data through tunnels. Also used to verify server port
  binding.
- **Usage in tests:** `nc -l 9000` to listen; `echo "hello" | nc localhost 8080`
  to send.

**Test Harness Equivalent: `test-tcp-peer`**
- **Required behavior:** Two modes: (1) *listen* — bind to a specified port,
  accept one connection, read data line-by-line printing to stdout, optionally
  echo back, exit on EOF or timeout. (2) *connect* — connect to host:port, send
  data from stdin or argument, print received data to stdout, exit on EOF or
  timeout.
- **Platform concerns:** Must handle both IPv4 and IPv6. On Windows, must not
  require WSL. Must support configurable timeouts to avoid hanging tests.

#### `curl` / `wget`

- **What:** HTTP client utilities.
- **Why:** Verify HTTP-level traffic through tunnels when a web server is the
  tunnel target.
- **Usage in tests:** `curl http://localhost:8080/` after opening a tunnel to a
  remote HTTP server.

**Test Harness Equivalent: `test-http-client`**
- **Required behavior:** Make GET/POST requests to a given URL, print status
  code and body to stdout, exit with non-zero on connection failure.
- **Platform concerns:** Must not depend on system SSL libraries for plain HTTP
  tests. Timeout support required.

#### `ssh-keygen`

- **What:** SSH key generation tool.
- **Why:** Generate test key pairs for SSH-based connection tests.
- **Usage in tests:** `ssh-keygen -t ed25519 -f /tmp/test_key -N ""`

**Test Harness Equivalent: `test-ssh-keygen`**
- **Required behavior:** Generate an Ed25519 key pair, write private key and
  `.pub` file to specified paths. No passphrase.
- **Platform concerns:** Must produce keys compatible with OpenSSH format on all
  platforms.

#### `sshd` (OpenSSH server)

- **What:** SSH daemon for accepting incoming SSH connections.
- **Why:** Required for all `launch`, `connect`, and `ssh` tests that use
  `ssh://` URIs against localhost.
- **Usage in tests:** Must be running and accepting connections on a known port.

**Test Harness Equivalent: `test-sshd`**
- **Required behavior:** Minimal SSH server that accepts key-based auth on a
  configurable port, provides shell access and command execution. Must support
  running as non-root on a non-privileged port.
- **Platform concerns:** Windows may need a dedicated OpenSSH server installation
  or a Rust-native SSH server implementation.

### 4.2 File Utilities

#### `cat`

- **What:** Concatenate and print file contents.
- **Why:** Verify file contents after remote write/copy operations.
- **Usage in tests:** `cat /tmp/remote_file.txt`

**Test Harness Equivalent: `test-file-read`**
- **Required behavior:** Read a file and print contents to stdout. Exit non-zero
  if file doesn't exist.
- **Platform concerns:** Must handle binary files without corruption on Windows.

#### `diff`

- **What:** Compare files line by line.
- **Why:** Verify that copied/transferred files match their originals.
- **Usage in tests:** `diff original.txt copied.txt`

**Test Harness Equivalent: `test-file-diff`**
- **Required behavior:** Compare two files byte-by-byte. Exit 0 if identical,
  exit 1 if different (printing first difference), exit 2 on error.
- **Platform concerns:** Binary-safe comparison. Handle line-ending differences
  configurable (strict or normalized).

#### `stat`

- **What:** Display file metadata (size, permissions, timestamps).
- **Why:** Verify file permissions after `fs set-permissions`, verify metadata
  after `fs metadata`.
- **Usage in tests:** `stat -f '%Lp' file` (macOS) or `stat -c '%a' file`
  (Linux)

**Test Harness Equivalent: `test-file-stat`**
- **Required behavior:** Print file size, permissions (octal on Unix, readonly
  flag on Windows), type (file/dir/symlink), and modification time in a
  consistent format.
- **Platform concerns:** Permission representation differs fundamentally between
  Unix (mode bits) and Windows (ACLs / readonly attribute).

#### `mkfifo`

- **What:** Create named pipes (FIFOs).
- **Why:** Test edge cases with special file types in filesystem operations.
- **Usage in tests:** `mkfifo /tmp/test_pipe`

**Test Harness Equivalent: `test-mkfifo`**
- **Required behavior:** Create a named pipe at the given path.
- **Platform concerns:** Named pipes work differently on Windows (named pipes
  are network-accessible, not filesystem objects). May need to be Unix-only or
  use a Windows equivalent.

#### `ln`

- **What:** Create symbolic and hard links.
- **Why:** Test symlink handling in `fs metadata`, `fs read`, `fs remove`,
  `fs set-permissions`.
- **Usage in tests:** `ln -s target linkname`

**Test Harness Equivalent: `test-symlink`**
- **Required behavior:** Create a symbolic link pointing to a target path.
- **Platform concerns:** Windows symlinks require `SeCreateSymbolicLinkPrivilege`
  or developer mode. The harness should detect and skip symlink tests on
  unprivileged Windows.

### 4.3 Process Utilities

#### `sleep`

- **What:** Pause execution for a specified duration.
- **Why:** Keep processes alive for a known duration during spawn/shell tests.
  Also used as a timer for server shutdown-policy tests.
- **Usage in tests:** `distant spawn -- sleep 5` to test process lifecycle.

**Test Harness Equivalent: `test-sleep`**
- **Required behavior:** Sleep for a specified number of seconds (or
  milliseconds), then exit 0.
- **Platform concerns:** Must be interruptible (exit on signal/termination).

#### `echo` / `printf`

- **What:** Print text to stdout.
- **Why:** Simplest command for verifying spawn/shell command execution.
- **Usage in tests:** `distant spawn -- echo "hello world"`

**Test Harness Equivalent:** Built into the harness directly (not a separate
binary). Any command that writes known output to stdout suffices.

#### `kill`

- **What:** Send signals to processes.
- **Why:** Terminate long-running processes (servers, watches) during tests.
  Test error recovery when server is killed mid-operation.
- **Usage in tests:** `kill <PID>` or `kill -9 <PID>`

**Test Harness Equivalent: `test-signal`**
- **Required behavior:** Send a specified signal to a PID.
- **Platform concerns:** Windows has no POSIX signals; use `taskkill /PID` or
  `TerminateProcess`. The harness should abstract this.

### 4.4 Text/Data Utilities

#### `jq`

- **What:** Command-line JSON processor.
- **Why:** Parse and validate `--format json` output from numerous commands.
- **Usage in tests:** `distant version --format json | jq '.server_version'`

**Test Harness Equivalent: `test-json-check`**
- **Required behavior:** Read JSON from stdin, extract a field by path, print
  its value. Exit non-zero on parse error or missing field.
- **Platform concerns:** None significant — pure data processing.

#### `base64`

- **What:** Base64 encode/decode utility.
- **Why:** Generate known binary content for `fs write` binary-data tests.
- **Usage in tests:** `echo "binary" | base64` to create test payloads.

**Test Harness Equivalent:** Built into harness (Rust's `base64` crate).

#### `wc`

- **What:** Word/line/byte count.
- **Why:** Verify file sizes after write operations, count directory entries.
- **Usage in tests:** `wc -c file` to check byte count.

**Test Harness Equivalent:** Built into harness (trivial in Rust).

### 4.5 Container Runtime

#### `docker` / `podman`

- **What:** Container runtime.
- **Why:** Required for `docker://` URI tests in `launch` and `connect`.
- **Usage in tests:** `distant launch docker://ubuntu:22.04`

**Test Harness Equivalent:** No replacement — tests requiring Docker must have
a real container runtime available. The harness should detect availability and
skip Docker tests when unavailable.

- **Platform availability:** Linux (native), macOS (Docker Desktop/colima),
  Windows (Docker Desktop with WSL2). Not typically available on FreeBSD/NetBSD/
  OpenBSD.

### 4.6 Service Management Verification

#### `systemctl` (Linux/systemd)

- **What:** systemd service controller.
- **Why:** Verify service install/start/stop/uninstall on systemd-based Linux.
- **Usage in tests:** `systemctl --user status distant-manager`

#### `launchctl` (macOS)

- **What:** launchd service controller.
- **Why:** Verify service operations on macOS.
- **Usage in tests:** `launchctl list | grep distant`

#### `sc` (Windows)

- **What:** Windows Service Control Manager CLI.
- **Why:** Verify service operations on Windows.
- **Usage in tests:** `sc query distant-manager`

#### `rc-service` / `service` (OpenRC / rc.d)

- **What:** OpenRC / rc.d service controllers.
- **Why:** Verify service operations on Alpine Linux, FreeBSD, NetBSD, OpenBSD.
- **Usage in tests:** `rc-service distant-manager status` or
  `service distant-manager status`

**Test Harness Equivalent:** No replacements for service management tools — these
are platform-specific by nature. The harness should detect the active service
manager and run the appropriate verification commands.

---

## 5. Command Test Sections

### 5A. Infrastructure

---

## Command: `distant generate config`

### Purpose

Generate a configuration file with base settings, printed to stdout or written
to a file.

### Dependencies

- `cat` — read generated config file when `--output` is used
- `diff` — compare generated configs

### Test Harness Equivalents

- **`test-file-read`** — read and print file contents
- **`test-file-diff`** — compare two files byte-by-byte

### Test Cases

#### TC-GEN-01: Generate config to stdout

**Category:** Happy Path
**Prerequisites:** None (no server or manager needed)
**Steps:**
1. Run `distant generate config`
**Expected Output:** TOML-formatted configuration printed to stdout with
default/commented settings.
**Verification:** Output is valid TOML (parseable by a TOML parser). Contains
expected section headers.
**Cleanup:** None

#### TC-GEN-02: Generate config to file

**Category:** Happy Path
**Prerequisites:** Writable temp directory
**Steps:**
1. Run `distant generate config --output /tmp/test_config.toml`
2. Read `/tmp/test_config.toml`
**Expected Output:** File created with same content as stdout generation.
**Verification:** `diff <(distant generate config) /tmp/test_config.toml`
produces no differences.
**Cleanup:** `rm /tmp/test_config.toml`

#### TC-GEN-03: Generate config to non-writable path

**Category:** Error Handling
**Prerequisites:** A path where the tester has no write permission
**Steps:**
1. Run `distant generate config --output /root/nope.toml` (as non-root)
**Expected Output:** Error message about permission denied.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant generate completion`

### Purpose

Generate shell completion scripts for a specified shell.

### Dependencies

- `cat` — read generated completion file when `--output` is used

### Test Harness Equivalents

- **`test-file-read`** — read and print file contents

### Test Cases

#### TC-GEN-04: Generate bash completions

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion bash`
**Expected Output:** Bash completion script printed to stdout. Contains
`complete` or `_distant` function definitions.
**Verification:** Output is non-empty and contains shell completion syntax.
**Cleanup:** None

#### TC-GEN-05: Generate zsh completions

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion zsh`
**Expected Output:** Zsh completion script with `#compdef distant` header.
**Verification:** Output begins with `#compdef` and is non-empty.
**Cleanup:** None

#### TC-GEN-06: Generate fish completions

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion fish`
**Expected Output:** Fish completion script using `complete -c distant` syntax.
**Verification:** Output contains `complete -c distant`.
**Cleanup:** None

#### TC-GEN-07: Generate PowerShell completions

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion powershell`
**Expected Output:** PowerShell completion script.
**Verification:** Output is non-empty and contains PowerShell syntax.
**Cleanup:** None

#### TC-GEN-08: Generate elvish completions

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion elvish`
**Expected Output:** Elvish completion script.
**Verification:** Output is non-empty.
**Cleanup:** None

#### TC-GEN-09: Generate completions to file

**Category:** Happy Path
**Prerequisites:** Writable temp directory
**Steps:**
1. Run `distant generate completion bash --output /tmp/distant.bash`
2. Read `/tmp/distant.bash`
**Expected Output:** File contains same content as stdout generation.
**Verification:** File exists and is non-empty.
**Cleanup:** `rm /tmp/distant.bash`

#### TC-GEN-10: Generate completions for invalid shell

**Category:** Error Handling
**Prerequisites:** None
**Steps:**
1. Run `distant generate completion notashell`
**Expected Output:** Error message indicating invalid shell value.
**Verification:** Exit code is non-zero. Error references valid values
(bash, elvish, fish, powershell, zsh).
**Cleanup:** None

---

## Command: `distant server listen`

### Purpose

Start a distant server that listens for incoming connections. This is the
server-side daemon that clients connect to.

### Dependencies

- `nc` — verify port binding by attempting connection
- `kill` — stop the server process
- `cat` — read log files
- `sleep` — test shutdown timers

### Test Harness Equivalents

- **`test-tcp-peer`** — verify server port is open by connecting
- **`test-signal`** — send termination signal to server PID
- **`test-file-read`** — read log output
- **`test-sleep`** — wait for shutdown policy timers

### Test Cases

#### TC-SRV-01: Start server with default options

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen` (in foreground)
2. Observe output
**Expected Output:** Server prints connection details (port, key) to stdout and
starts listening. Output includes a line with the port number and auth key.
**Verification:** Another terminal can connect to the printed port.
**Cleanup:** Ctrl+C or kill the process.

#### TC-SRV-02: Start server on specific port

**Category:** Happy Path
**Prerequisites:** Port 8899 is free
**Steps:**
1. Run `distant server listen --port 8899`
**Expected Output:** Server binds to port 8899.
**Verification:** Output indicates port 8899. `nc -z localhost 8899` succeeds.
**Cleanup:** Kill the server.

#### TC-SRV-03: Start server on port range

**Category:** Happy Path
**Prerequisites:** At least one port in range 9000-9010 is free
**Steps:**
1. Run `distant server listen --port 9000:9010`
**Expected Output:** Server binds to first available port in range 9000-9010.
**Verification:** Output indicates a port within the range.
**Cleanup:** Kill the server.

#### TC-SRV-04: Start server on port 0 (OS-assigned)

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --port 0`
**Expected Output:** Server binds to an OS-assigned ephemeral port.
**Verification:** Output indicates a non-zero port number.
**Cleanup:** Kill the server.

#### TC-SRV-05: Start server on already-bound port

**Category:** Error Handling
**Prerequisites:** Port 8899 is already in use (e.g., `nc -l 8899 &`)
**Steps:**
1. Run `distant server listen --port 8899`
**Expected Output:** Error about address already in use.
**Verification:** Exit code is non-zero.
**Cleanup:** Kill the `nc` process.

#### TC-SRV-06: Start server with --host any

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --host any`
**Expected Output:** Server binds to 0.0.0.0 (all interfaces).
**Verification:** Server accepts connections.
**Cleanup:** Kill the server.

#### TC-SRV-07: Start server with --host ssh

**Category:** Happy Path
**Prerequisites:** `SSH_CONNECTION` environment variable set (or running in an
SSH session)
**Steps:**
1. Run `distant server listen --host ssh`
**Expected Output:** If `SSH_CONNECTION` is set, server binds to the IP from
that variable. If not set, falls back to default behavior.
**Verification:** Server starts and reports its bind address.
**Cleanup:** Kill the server.

#### TC-SRV-08: Start server with --host IP

**Category:** Happy Path
**Prerequisites:** 127.0.0.1 is available
**Steps:**
1. Run `distant server listen --host 127.0.0.1`
**Expected Output:** Server binds specifically to 127.0.0.1.
**Verification:** `nc -z 127.0.0.1 <PORT>` succeeds.
**Cleanup:** Kill the server.

#### TC-SRV-09: Start server with IPv6

**Category:** Happy Path
**Prerequisites:** IPv6 is enabled on the machine
**Steps:**
1. Run `distant server listen --host any --use-ipv6`
**Expected Output:** Server binds to `[::]` (IPv6 any).
**Verification:** Server starts without error. IPv6 connections accepted.
**Cleanup:** Kill the server.

#### TC-SRV-10: Start server with --daemon

**Category:** Happy Path
**Prerequisites:** Unix platform
**Steps:**
1. Run `distant server listen --daemon`
**Expected Output:** Process forks to background. Connection details printed to
stdout before the foreground process exits.
**Verification:** `ps aux | grep distant` shows a server process running.
**Cleanup:** Kill the daemon PID.

#### TC-SRV-11: Start server with shutdown=after

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --shutdown after=5`
2. Wait 6 seconds
**Expected Output:** Server shuts down automatically after 5 seconds.
**Verification:** Process exits on its own after the timeout.
**Cleanup:** None (self-cleans).

#### TC-SRV-12: Start server with shutdown=lonely

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --shutdown lonely=10`
2. Do not connect any clients
3. Wait 11 seconds
**Expected Output:** Server shuts down after 10 seconds with no connections.
**Verification:** Process exits on its own.
**Cleanup:** None (self-cleans).

#### TC-SRV-13: Start server with shutdown=never

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --shutdown never`
2. Wait an arbitrary amount of time
**Expected Output:** Server remains running indefinitely.
**Verification:** Server process still alive after waiting.
**Cleanup:** Kill the server.

#### TC-SRV-14: Start server with --current-dir

**Category:** Happy Path
**Prerequisites:** `/tmp` exists
**Steps:**
1. Run `distant server listen --current-dir /tmp`
**Expected Output:** Server starts with `/tmp` as its working directory.
**Verification:** Connecting and running `pwd` (via spawn) returns `/tmp`.
**Cleanup:** Kill the server.

#### TC-SRV-15: Start server with --key-from-stdin

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Generate 32 bytes: `head -c 32 /dev/urandom > /tmp/test_key.bin`
2. Run `cat /tmp/test_key.bin | distant server listen --key-from-stdin`
**Expected Output:** Server starts using the provided 32-byte key.
**Verification:** Server starts without error.
**Cleanup:** Kill the server. `rm /tmp/test_key.bin`

#### TC-SRV-16: Start server with --key-from-stdin (too few bytes)

**Category:** Error Handling
**Prerequisites:** None
**Steps:**
1. Run `echo "short" | distant server listen --key-from-stdin`
**Expected Output:** Error about insufficient key bytes (fewer than 32).
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-SRV-17: Start server with watch polling options

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --watch-polling --watch-poll-interval 2`
**Expected Output:** Server starts using polling-based file watcher with 2s
interval.
**Verification:** Server starts without error. File watching (tested via
`fs watch`) uses polling.
**Cleanup:** Kill the server.

#### TC-SRV-18: Start server with custom debounce timeout

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant server listen --watch-debounce-timeout 2.0`
**Expected Output:** Server starts with 2-second debounce window.
**Verification:** Server starts without error.
**Cleanup:** Kill the server.

---

## Command: `distant manager listen`

### Purpose

Start the manager process that coordinates connections between clients and
servers. The manager listens on a Unix socket (Unix) or named pipe (Windows).

### Dependencies

- `kill` — stop the manager process
- Platform service verification tools (see section 4.6)

### Test Harness Equivalents

- **`test-signal`** — terminate manager process

### Test Cases

#### TC-MGR-01: Start manager with default options

**Category:** Happy Path
**Prerequisites:** No manager already running on the default socket
**Steps:**
1. Run `distant manager listen` (foreground)
**Expected Output:** Manager starts listening, process stays in foreground.
**Verification:** `distant manager version` in another terminal succeeds.
**Cleanup:** Ctrl+C or kill.

#### TC-MGR-02: Start manager as daemon

**Category:** Happy Path
**Prerequisites:** Unix platform. No manager already running.
**Steps:**
1. Run `distant manager listen --daemon`
**Expected Output:** Process forks to background. Foreground returns.
**Verification:** `distant manager version` succeeds.
**Cleanup:** Kill the daemon PID.

#### TC-MGR-03: Start manager with --access owner

**Category:** Happy Path
**Prerequisites:** Unix platform
**Steps:**
1. Run `distant manager listen --access owner --daemon`
**Expected Output:** Manager creates socket with `0600` permissions.
**Verification:** `stat` on the socket file shows owner-only permissions.
Other users cannot connect.
**Cleanup:** Kill the daemon.

#### TC-MGR-04: Start manager with --access group

**Category:** Happy Path
**Prerequisites:** Unix platform
**Steps:**
1. Run `distant manager listen --access group --daemon`
**Expected Output:** Manager creates socket with `0660` permissions.
**Verification:** `stat` on the socket file shows owner+group permissions.
**Cleanup:** Kill the daemon.

#### TC-MGR-05: Start manager with --access anyone

**Category:** Happy Path
**Prerequisites:** Unix platform
**Steps:**
1. Run `distant manager listen --access anyone --daemon`
**Expected Output:** Manager creates socket with `0666` permissions.
**Verification:** `stat` on the socket file shows world-readable/writable.
**Cleanup:** Kill the daemon.

#### TC-MGR-06: Start manager with --user

**Category:** Happy Path
**Prerequisites:** None
**Steps:**
1. Run `distant manager listen --user --daemon`
**Expected Output:** Manager listens on a user-local socket/pipe.
**Verification:** `distant manager version` succeeds when using the
user-local socket.
**Cleanup:** Kill the daemon.

#### TC-MGR-07: Start manager with --plugin

**Category:** Happy Path
**Prerequisites:** A plugin binary exists at a known path
**Steps:**
1. Run `distant manager listen --plugin myplugin=/path/to/plugin --daemon`
**Expected Output:** Manager registers the plugin as a connection handler.
**Verification:** Manager starts without error. Plugin scheme is available
for `launch`/`connect`.
**Cleanup:** Kill the daemon.

#### TC-MGR-08: Start manager with custom Unix socket path

**Category:** Happy Path
**Prerequisites:** Unix platform
**Steps:**
1. Run `distant manager listen --unix-socket /tmp/test_distant.sock --daemon`
**Expected Output:** Manager creates socket at `/tmp/test_distant.sock`.
**Verification:** `distant manager version --unix-socket /tmp/test_distant.sock`
succeeds.
**Cleanup:** Kill the daemon. `rm /tmp/test_distant.sock`

#### TC-MGR-09: Start manager when already running

**Category:** Error Handling
**Prerequisites:** Manager already running on default socket
**Steps:**
1. Run `distant manager listen`
**Expected Output:** Error about socket/pipe already in use or manager already
running.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant manager version`

### Purpose

Retrieve the manager's version and capability information.

### Dependencies

- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-json-check`** — validate JSON output structure

### Test Cases

#### TC-MGR-10: Get manager version (shell format)

**Category:** Happy Path
**Prerequisites:** Manager is running
**Steps:**
1. Run `distant manager version`
**Expected Output:** Human-readable version and capabilities list.
**Verification:** Output is non-empty and contains version information.
**Cleanup:** None

#### TC-MGR-11: Get manager version (JSON format)

**Category:** Happy Path
**Prerequisites:** Manager is running
**Steps:**
1. Run `distant manager version --format json`
**Expected Output:** JSON object with version and capabilities.
**Verification:** Output is valid JSON. `echo '...' | jq .` succeeds.
**Cleanup:** None

#### TC-MGR-12: Get manager version when no manager running

**Category:** Error Handling
**Prerequisites:** No manager running
**Steps:**
1. Run `distant manager version`
**Expected Output:** Error about connection refused or no manager.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant manager service {install,uninstall,start,stop}`

### Purpose

Install, uninstall, start, and stop the distant manager as a system or user
service using the platform's native service manager.

### Dependencies

- Platform-specific service management tools (see section 4.6)

### Test Harness Equivalents

No cross-platform replacements — these tests use native service tools for
verification.

### Test Cases

#### TC-SVC-01: Install as system service (auto-detect kind)

**Category:** Happy Path
**Prerequisites:** Elevated privileges (root/admin). No existing service.
**Steps:**
1. Run `distant manager service install` (with appropriate privileges)
**Expected Output:** Service installed successfully.
**Verification:**
- **Linux (systemd):** `systemctl status distant-manager` shows loaded
- **macOS:** `launchctl list | grep distant` shows entry
- **Windows:** `sc query distant-manager` shows installed
- **FreeBSD/NetBSD/OpenBSD:** `service distant-manager status` recognizes it
**Cleanup:** `distant manager service uninstall`

#### TC-SVC-02: Install as user service

**Category:** Happy Path
**Prerequisites:** No elevated privileges needed
**Steps:**
1. Run `distant manager service install --user`
**Expected Output:** User-level service installed.
**Verification:**
- **Linux (systemd):** `systemctl --user status distant-manager` shows loaded
- **macOS:** `launchctl list | grep distant` in user domain
**Cleanup:** `distant manager service uninstall --user`

#### TC-SVC-03: Install with explicit kind

**Category:** Happy Path
**Prerequisites:** Platform supports the specified kind. Elevated privileges.
**Steps:**
1. Run `distant manager service install --kind systemd` (Linux)
   or `distant manager service install --kind launchd` (macOS)
   or `distant manager service install --kind sc` (Windows)
**Expected Output:** Service installed using the specified service manager.
**Verification:** Corresponding service tool confirms installation.
**Cleanup:** `distant manager service uninstall --kind <same>`

#### TC-SVC-04: Install with additional manager args

**Category:** Happy Path
**Prerequisites:** Elevated privileges
**Steps:**
1. Run `distant manager service install -- --access anyone`
**Expected Output:** Service installed. When started, manager uses
`--access anyone`.
**Verification:** Install succeeds. Starting the service and checking socket
permissions confirms the args were passed.
**Cleanup:** `distant manager service stop && distant manager service uninstall`

#### TC-SVC-05: Start installed service

**Category:** Happy Path
**Prerequisites:** Service is installed (TC-SVC-01 or TC-SVC-02)
**Steps:**
1. Run `distant manager service start`
**Expected Output:** Service starts.
**Verification:** `distant manager version` succeeds. Platform service tool
shows running state.
**Cleanup:** `distant manager service stop`

#### TC-SVC-06: Stop running service

**Category:** Happy Path
**Prerequisites:** Service is running (TC-SVC-05)
**Steps:**
1. Run `distant manager service stop`
**Expected Output:** Service stops.
**Verification:** `distant manager version` fails (no manager). Platform
service tool shows stopped state.
**Cleanup:** None

#### TC-SVC-07: Uninstall service

**Category:** Happy Path
**Prerequisites:** Service is installed and stopped
**Steps:**
1. Run `distant manager service uninstall`
**Expected Output:** Service removed from service manager.
**Verification:** Platform service tool no longer recognizes the service.
**Cleanup:** None

#### TC-SVC-08: Start service that is not installed

**Category:** Error Handling
**Prerequisites:** Service is not installed
**Steps:**
1. Run `distant manager service start`
**Expected Output:** Error about service not found or not installed.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-SVC-09: Uninstall service that is not installed

**Category:** Error Handling
**Prerequisites:** Service is not installed
**Steps:**
1. Run `distant manager service uninstall`
**Expected Output:** Error or informational message about service not found.
**Verification:** Exit code is non-zero (or zero with warning).
**Cleanup:** None

#### TC-SVC-10: Install with wrong kind for platform

**Category:** Error Handling
**Prerequisites:** Running on macOS
**Steps:**
1. Run `distant manager service install --kind systemd`
**Expected Output:** Error about unsupported service manager for this platform.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

### 5B. Connection Management

---

## Command: `distant launch`

### Purpose

Launch a distant server on a remote machine via SSH (or Docker) and establish
a connection through the manager. This is the primary way to start a remote
server.

### Dependencies

- `sshd` — local SSH server for self-to-self testing
- SSH key pair — for passwordless authentication
- `docker` — for Docker URI tests
- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-sshd`** — SSH server for localhost testing
- **`test-ssh-keygen`** — generate test keys
- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-LCH-01: Launch via SSH to localhost

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd` accepting key-based auth.
**Steps:**
1. Run `distant launch ssh://localhost`
**Expected Output:** Connection ID printed (e.g., a numeric or UUID identifier).
Server is started on the remote (self) and connection established.
**Verification:** `distant status` shows an active connection.
**Cleanup:** `distant kill`

#### TC-LCH-02: Launch via SSH with explicit user and port

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd` on port 22.
**Steps:**
1. Run `distant launch ssh://$(whoami)@localhost:22`
**Expected Output:** Connection ID printed. Same behavior as TC-LCH-01.
**Verification:** `distant status` shows connection with user and port details.
**Cleanup:** `distant kill`

#### TC-LCH-03: Launch with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --format json ssh://localhost`
**Expected Output:** JSON object containing connection ID and destination info.
**Verification:** Output is valid JSON. Contains connection identifier.
**Cleanup:** `distant kill`

#### TC-LCH-04: Launch with custom distant binary path

**Category:** Happy Path
**Prerequisites:** Manager running. `distant` binary available at a known
absolute path on localhost (e.g., `/usr/local/bin/distant`).
**Steps:**
1. Run `distant launch --distant /usr/local/bin/distant ssh://localhost`
**Expected Output:** Connection established using the specified distant path.
**Verification:** `distant status` shows active connection.
**Cleanup:** `distant kill`

#### TC-LCH-05: Launch with --distant-bind-server ssh

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --distant-bind-server ssh ssh://localhost`
**Expected Output:** Server binds to the IP from `SSH_CONNECTION` env var.
**Verification:** Connection established.
**Cleanup:** `distant kill`

#### TC-LCH-06: Launch with --distant-bind-server any

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --distant-bind-server any ssh://localhost`
**Expected Output:** Server binds to default interface (0.0.0.0).
**Verification:** Connection established.
**Cleanup:** `distant kill`

#### TC-LCH-07: Launch with --distant-bind-server IP

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --distant-bind-server 127.0.0.1 ssh://localhost`
**Expected Output:** Server binds specifically to 127.0.0.1.
**Verification:** Connection established.
**Cleanup:** `distant kill`

#### TC-LCH-08: Launch with additional distant args

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --distant-args "--port 9999" ssh://localhost`
**Expected Output:** Server started with the provided port argument.
**Verification:** Connection established. Server is on port 9999.
**Cleanup:** `distant kill`

#### TC-LCH-09: Launch with connection options

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant launch --options 'key="value"' ssh://localhost`
**Expected Output:** Connection established with the options passed to the
handler.
**Verification:** `distant status` shows active connection.
**Cleanup:** `distant kill`

#### TC-LCH-10: Launch via Docker URI

**Category:** Happy Path
**Prerequisites:** Manager running. Docker runtime available and running.
**Steps:**
1. Run `distant launch docker://ubuntu:22.04`
**Expected Output:** Docker container started with distant server running
inside. Connection ID returned.
**Verification:** `distant status` shows connection to Docker destination.
**Cleanup:** `distant kill` (container should also be cleaned up).

#### TC-LCH-11: Launch to unreachable host

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant launch ssh://192.0.2.1` (TEST-NET, unreachable)
**Expected Output:** Error about connection timeout or host unreachable.
**Verification:** Exit code is non-zero. No connection created.
**Cleanup:** None

#### TC-LCH-12: Launch with invalid URI scheme

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant launch ftp://localhost`
**Expected Output:** Error about unsupported or unknown scheme.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-LCH-13: Launch when no manager running

**Category:** Error Handling
**Prerequisites:** No manager running.
**Steps:**
1. Run `distant launch ssh://localhost`
**Expected Output:** Error about unable to connect to manager.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant connect`

### Purpose

Connect the manager to an already-running distant server at the specified
destination.

### Dependencies

- `sshd` — for SSH-based connections
- `jq` — parse JSON output

### Test Harness Equivalents

- **`test-sshd`** — SSH server for localhost testing
- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-CON-01: Connect to SSH destination

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant connect ssh://localhost`
**Expected Output:** Connection ID printed.
**Verification:** `distant status` shows active connection.
**Cleanup:** `distant kill`

#### TC-CON-02: Connect with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant connect --format json ssh://localhost`
**Expected Output:** JSON object with connection details.
**Verification:** Valid JSON output. Contains connection ID.
**Cleanup:** `distant kill`

#### TC-CON-03: Connect with --new (force new connection)

**Category:** Happy Path
**Prerequisites:** Manager running. Existing connection to `ssh://localhost`.
**Steps:**
1. First: `distant connect ssh://localhost` (creates connection A)
2. Run `distant connect --new ssh://localhost`
**Expected Output:** New connection ID, different from connection A.
**Verification:** `distant status` shows two active connections.
**Cleanup:** Kill both connections.

#### TC-CON-04: Connect to same destination without --new

**Category:** Happy Path
**Prerequisites:** Manager running. Existing connection to `ssh://localhost`.
**Steps:**
1. First: `distant connect ssh://localhost` (creates connection A)
2. Run `distant connect ssh://localhost` (without --new)
**Expected Output:** May reuse existing connection or create new one (behavior
depends on implementation). Connection ID returned.
**Verification:** `distant status` shows connection(s).
**Cleanup:** Kill all connections.

#### TC-CON-05: Connect with options

**Category:** Happy Path
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Run `distant connect --options 'key="value"' ssh://localhost`
**Expected Output:** Connection established.
**Verification:** `distant status` shows active connection.
**Cleanup:** `distant kill`

#### TC-CON-06: Connect via Docker URI

**Category:** Happy Path
**Prerequisites:** Manager running. Docker runtime available.
**Steps:**
1. Run `distant connect docker://ubuntu:22.04`
**Expected Output:** Connection established to Docker container.
**Verification:** `distant status` shows Docker connection.
**Cleanup:** `distant kill`

#### TC-CON-07: Connect to unreachable destination

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant connect ssh://192.0.2.1`
**Expected Output:** Error about connection failure.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-CON-08: Connect when no manager running

**Category:** Error Handling
**Prerequisites:** No manager running.
**Steps:**
1. Run `distant connect ssh://localhost`
**Expected Output:** Error about unable to connect to manager.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant ssh`

### Purpose

Connect to a remote host via SSH and optionally open a shell or run a command.
This is the simplest way to use distant — it auto-starts the manager, connects
via SSH (no distant binary needed on the remote), and opens a shell or runs the
specified command.

### Dependencies

- `sshd` — local SSH server for self-to-self testing
- SSH key pair — for passwordless authentication

### Test Harness Equivalents

- **`test-sshd`** — SSH server for localhost testing
- **`test-ssh-keygen`** — generate test keys

### Test Cases

#### TC-SSH-01: Open interactive shell

**Category:** Happy Path
**Prerequisites:** Local `sshd` accepting key-based auth. Manager may or may
not be running (auto-started).
**Steps:**
1. Run `distant ssh localhost`
**Expected Output:**

```
┌─────────────────────────────────┐
│ $ distant ssh localhost          │
│ user@localhost:~$               │
│                                  │
│ $ echo "hello from ssh"         │
│ hello from ssh                   │
│ $ exit                           │
│                                  │
└─────────────────────────────────┘
```

**Verification:** Interactive shell prompt appears. Commands execute and produce
output. `exit` returns to local shell.
**Cleanup:** None (shell exited cleanly).

#### TC-SSH-02: Run a single command

**Category:** Happy Path
**Prerequisites:** Local `sshd`. Manager auto-starts.
**Steps:**
1. Run `distant ssh localhost -- echo "hello world"`
**Expected Output:** `hello world` printed to stdout.
**Verification:** Output matches expected string. Exit code is 0.
**Cleanup:** None

#### TC-SSH-03: Run command with arguments

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh localhost -- ls -la /tmp`
**Expected Output:** Detailed listing of `/tmp` directory.
**Verification:** Output shows typical `ls -la` format.
**Cleanup:** None

#### TC-SSH-04: SSH with explicit user and port

**Category:** Happy Path
**Prerequisites:** Local `sshd` on port 22.
**Steps:**
1. Run `distant ssh $(whoami)@localhost:22 -- whoami`
**Expected Output:** Current username printed.
**Verification:** Output matches `$(whoami)`.
**Cleanup:** None

#### TC-SSH-05: SSH with environment variables

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --environment 'MY_VAR=hello' localhost -- printenv MY_VAR`
**Expected Output:** `hello` printed.
**Verification:** Environment variable was set on remote.
**Cleanup:** None

#### TC-SSH-06: SSH with --current-dir

**Category:** Happy Path
**Prerequisites:** Local `sshd`. `/tmp` exists.
**Steps:**
1. Run `distant ssh --current-dir /tmp localhost -- pwd`
**Expected Output:** `/tmp` printed (or platform equivalent).
**Verification:** Working directory was changed.
**Cleanup:** None

#### TC-SSH-07: SSH with --predict off

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --predict off localhost`
**Expected Output:** Interactive shell opens. All output comes from server
(no local prediction).
**Verification:** Shell functions normally. No local echo artifacts.
**Cleanup:** `exit`

#### TC-SSH-08: SSH with --predict on

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --predict on localhost`
**Expected Output:** Interactive shell opens. Keystrokes are echoed locally
immediately before server confirmation.
**Verification:** Typing feels responsive. Characters appear before round-trip.
**Cleanup:** `exit`

#### TC-SSH-09: SSH with --predict adaptive

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --predict adaptive localhost`
**Expected Output:** Interactive shell opens. Prediction activates automatically
if SRTT exceeds 30ms.
**Verification:** Shell functions normally. On localhost (low latency),
prediction may not activate.
**Cleanup:** `exit`

#### TC-SSH-10: SSH with --predict fast

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --predict fast localhost`
**Expected Output:** Interactive shell opens. All keystrokes echoed locally,
predictions display immediately after epoch boundaries.
**Verification:** Shell functions normally with aggressive prediction.
**Cleanup:** `exit`

#### TC-SSH-11: SSH with --predict fast-adaptive

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --predict fast-adaptive localhost`
**Expected Output:** Interactive shell opens. Adaptive prediction with fast
epoch confirmation.
**Verification:** Shell functions normally.
**Cleanup:** `exit`

#### TC-SSH-12: SSH with --new (force new connection)

**Category:** Happy Path
**Prerequisites:** Manager running with existing SSH connection.
**Steps:**
1. Run `distant ssh --new localhost -- echo "new connection"`
**Expected Output:** `new connection` printed. A new connection was created
even though one already existed.
**Verification:** `distant status` shows an additional connection.
**Cleanup:** `distant kill`

#### TC-SSH-13: SSH with options

**Category:** Happy Path
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh --options 'key="value"' localhost -- echo ok`
**Expected Output:** `ok` printed. Options were passed to SSH handler.
**Verification:** Exit code 0.
**Cleanup:** None

#### TC-SSH-14: SSH to unreachable host

**Category:** Error Handling
**Prerequisites:** None.
**Steps:**
1. Run `distant ssh 192.0.2.1 -- echo hi`
**Expected Output:** Error about connection timeout or host unreachable.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-SSH-15: SSH with command that exits non-zero

**Category:** Edge Case
**Prerequisites:** Local `sshd`.
**Steps:**
1. Run `distant ssh localhost -- false`
**Expected Output:** No stdout. Exit code reflects the remote command's exit
code (1 for `false`).
**Verification:** `echo $?` after the command shows non-zero.
**Cleanup:** None

---

## Command: `distant status`

### Purpose

Show the current status of the manager and active connections. With no
arguments, shows an overview. With a connection ID, shows detailed info about
that specific connection.

### Dependencies

- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-STA-01: Status overview with active connections

**Category:** Happy Path
**Prerequisites:** Manager running. At least one connection active.
**Steps:**
1. Run `distant status`
**Expected Output:** Overview showing manager info and list of active
connections with IDs, destinations, and status.
**Verification:** Output lists the active connection(s).
**Cleanup:** None

#### TC-STA-02: Status overview with no connections

**Category:** Happy Path
**Prerequisites:** Manager running. No active connections.
**Steps:**
1. Run `distant status`
**Expected Output:** Overview showing manager info but no active connections
(empty list or "no connections" message).
**Verification:** Output indicates no active connections.
**Cleanup:** None

#### TC-STA-03: Status for specific connection ID

**Category:** Happy Path
**Prerequisites:** Manager running. At least one connection active. Note its ID.
**Steps:**
1. Run `distant status <ID>`
**Expected Output:** Detailed information about the specific connection
including destination, uptime, and protocol details.
**Verification:** Output shows details matching the connection.
**Cleanup:** None

#### TC-STA-04: Status for invalid connection ID

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant status 999999`
**Expected Output:** Error about connection not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-STA-05: Status with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. At least one connection active.
**Steps:**
1. Run `distant status --format json`
**Expected Output:** JSON object with manager and connection information.
**Verification:** Valid JSON. Contains connections array.
**Cleanup:** None

#### TC-STA-06: Status for specific ID with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. Active connection with known ID.
**Steps:**
1. Run `distant status --format json <ID>`
**Expected Output:** JSON object with detailed connection info.
**Verification:** Valid JSON. Contains connection details.
**Cleanup:** None

#### TC-STA-07: Status when no manager running

**Category:** Error Handling
**Prerequisites:** No manager running.
**Steps:**
1. Run `distant status`
**Expected Output:** Error about unable to connect to manager.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant kill`

### Purpose

Kill (disconnect) an active connection by ID, or interactively select one to
kill if no ID is provided.

### Dependencies

- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-KIL-01: Kill connection by ID

**Category:** Happy Path
**Prerequisites:** Manager running. At least one active connection. Note its ID.
**Steps:**
1. Run `distant kill <ID>`
**Expected Output:** Confirmation that the connection was killed.
**Verification:** `distant status` no longer shows this connection.
**Cleanup:** None

#### TC-KIL-02: Kill with interactive prompt

**Category:** Happy Path
**Prerequisites:** Manager running. At least one active connection.
**Steps:**
1. Run `distant kill` (no ID argument)
**Expected Output:** Interactive prompt listing available connections and
asking which to kill.
**Verification:** After selecting a connection, it is killed. `distant status`
confirms.
**Cleanup:** None

#### TC-KIL-03: Kill nonexistent connection ID

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant kill 999999`
**Expected Output:** Error about connection not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-KIL-04: Kill with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. Active connection with known ID.
**Steps:**
1. Run `distant kill --format json <ID>`
**Expected Output:** JSON confirmation of the kill operation.
**Verification:** Valid JSON output. Connection is killed.
**Cleanup:** None

#### TC-KIL-05: Kill when no connections exist

**Category:** Edge Case
**Prerequisites:** Manager running. No active connections.
**Steps:**
1. Run `distant kill`
**Expected Output:** Message indicating no connections to kill, or empty
interactive prompt.
**Verification:** Graceful handling — no crash.
**Cleanup:** None

---

## Command: `distant select`

### Purpose

Select the active connection that subsequent commands will use by default.

### Dependencies

- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-SEL-01: Select connection by ID

**Category:** Happy Path
**Prerequisites:** Manager running. At least two active connections. Note their
IDs.
**Steps:**
1. Run `distant select <ID_B>`
**Expected Output:** Confirmation that connection B is now active.
**Verification:** Subsequent commands (e.g., `distant version`) use connection B.
**Cleanup:** None

#### TC-SEL-02: Select with interactive prompt

**Category:** Happy Path
**Prerequisites:** Manager running. At least two active connections.
**Steps:**
1. Run `distant select` (no argument)
**Expected Output:** Interactive prompt listing available connections.
**Verification:** After selecting one, it becomes the active connection.
**Cleanup:** None

#### TC-SEL-03: Select nonexistent connection ID

**Category:** Error Handling
**Prerequisites:** Manager running.
**Steps:**
1. Run `distant select 999999`
**Expected Output:** Error about connection not found.
**Verification:** Exit code is non-zero. Active connection unchanged.
**Cleanup:** None

#### TC-SEL-04: Select with JSON format

**Category:** Happy Path
**Prerequisites:** Manager running. Multiple active connections.
**Steps:**
1. Run `distant select --format json <ID>`
**Expected Output:** JSON confirmation.
**Verification:** Valid JSON. Active connection changed.
**Cleanup:** None

#### TC-SEL-05: Select when only one connection exists

**Category:** Edge Case
**Prerequisites:** Manager running. Exactly one active connection.
**Steps:**
1. Run `distant select`
**Expected Output:** Either auto-selects the only connection or shows a
single-item prompt.
**Verification:** The single connection remains active.
**Cleanup:** None

---

### 5C. File Operations

---

## Command: `distant copy`

### Purpose

Copy files between local and remote machines. Remote paths are prefixed with
`:` to distinguish them from local paths. Exactly one of source or destination
must be remote.

### Dependencies

- `cat` — read file contents for verification
- `diff` — compare source and copied files
- `stat` — check file metadata
- `mkdir` — create test directories
- `ln` — create symlinks for edge cases

### Test Harness Equivalents

- **`test-file-read`** — read and print file contents
- **`test-file-diff`** — compare two files byte-by-byte
- **`test-file-stat`** — check file metadata
- **`test-symlink`** — create symbolic links

### Test Cases

#### TC-CPY-01: Upload a single file

**Category:** Happy Path
**Prerequisites:** Active connection. Local file `/tmp/test_upload.txt` with
known content.
**Steps:**
1. Create local file: `echo "upload test" > /tmp/test_upload.txt`
2. Run `distant copy /tmp/test_upload.txt :/tmp/test_upload_remote.txt`
3. Verify: `distant fs read /tmp/test_upload_remote.txt`
**Expected Output:** No error from copy. Remote file contains "upload test".
**Verification:** `distant fs read` returns matching content.
**Cleanup:** `distant fs remove /tmp/test_upload_remote.txt`
`rm /tmp/test_upload.txt`

#### TC-CPY-02: Download a single file

**Category:** Happy Path
**Prerequisites:** Active connection. Remote file exists.
**Steps:**
1. Create remote file: `distant fs write /tmp/test_download.txt "download test"`
2. Run `distant copy :/tmp/test_download.txt /tmp/test_download_local.txt`
3. Read local file: `cat /tmp/test_download_local.txt`
**Expected Output:** Local file contains "download test".
**Verification:** File contents match.
**Cleanup:** `distant fs remove /tmp/test_download.txt`
`rm /tmp/test_download_local.txt`

#### TC-CPY-03: Upload a directory recursively

**Category:** Happy Path
**Prerequisites:** Active connection. Local directory with files.
**Steps:**
1. Create local dir: `mkdir -p /tmp/test_dir && echo "a" > /tmp/test_dir/a.txt && echo "b" > /tmp/test_dir/b.txt`
2. Run `distant copy -r /tmp/test_dir :/tmp/test_dir_remote`
3. Verify: `distant fs read /tmp/test_dir_remote`
**Expected Output:** Remote directory contains `a.txt` and `b.txt`.
**Verification:** `distant fs read /tmp/test_dir_remote` lists both files.
`distant fs read /tmp/test_dir_remote/a.txt` returns "a".
**Cleanup:** `distant fs remove --force /tmp/test_dir_remote`
`rm -rf /tmp/test_dir`

#### TC-CPY-04: Download a directory recursively

**Category:** Happy Path
**Prerequisites:** Active connection. Remote directory with files.
**Steps:**
1. Setup remote dir:
   `distant fs make-dir --all /tmp/test_dl_dir`
   `distant fs write /tmp/test_dl_dir/x.txt "x"`
2. Run `distant copy -r :/tmp/test_dl_dir /tmp/test_dl_dir_local`
3. Read local: `cat /tmp/test_dl_dir_local/x.txt`
**Expected Output:** Local directory with "x" in x.txt.
**Verification:** File contents match.
**Cleanup:** `distant fs remove --force /tmp/test_dl_dir`
`rm -rf /tmp/test_dl_dir_local`

#### TC-CPY-05: Upload directory without -r flag

**Category:** Error Handling
**Prerequisites:** Active connection. Local directory exists.
**Steps:**
1. Create local dir: `mkdir -p /tmp/test_dir_no_r`
2. Run `distant copy /tmp/test_dir_no_r :/tmp/dest`
**Expected Output:** Error indicating that recursive flag is needed for
directories.
**Verification:** Exit code is non-zero.
**Cleanup:** `rm -rf /tmp/test_dir_no_r`

#### TC-CPY-06: Copy with nonexistent source

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant copy /tmp/does_not_exist.txt :/tmp/dest.txt`
**Expected Output:** Error about source not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-CPY-07: Copy with both paths local

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant copy /tmp/a.txt /tmp/b.txt` (no `:` prefix on either)
**Expected Output:** Error indicating exactly one path must be remote.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-CPY-08: Copy with both paths remote

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant copy :/tmp/a.txt :/tmp/b.txt`
**Expected Output:** Error indicating exactly one path must be remote.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-CPY-09: Upload to non-writable remote path

**Category:** Error Handling
**Prerequisites:** Active connection. Remote path that is not writable.
**Steps:**
1. Create local file: `echo "test" > /tmp/perm_test.txt`
2. Run `distant copy /tmp/perm_test.txt :/root/no_write.txt` (as non-root)
**Expected Output:** Error about permission denied.
**Verification:** Exit code is non-zero.
**Cleanup:** `rm /tmp/perm_test.txt`

#### TC-CPY-10: Copy with --connection flag

**Category:** Happy Path
**Prerequisites:** Manager running. Multiple connections. File exists locally.
**Steps:**
1. Run `distant copy --connection <ID> /tmp/local.txt :/tmp/remote.txt`
**Expected Output:** File copied using the specified connection.
**Verification:** `distant fs read --connection <ID> /tmp/remote.txt` shows
content.
**Cleanup:** `distant fs remove --connection <ID> /tmp/remote.txt`

---

## Command: `distant fs copy`

### Purpose

Copy a file or directory on the remote machine (remote-to-remote copy within
the same server).

### Dependencies

- `diff` — compare files (indirectly via `fs read`)

### Test Harness Equivalents

- **`test-file-diff`** — compare two files

### Test Cases

#### TC-FSC-01: Copy a remote file

**Category:** Happy Path
**Prerequisites:** Active connection. Remote file `/tmp/orig.txt` exists.
**Steps:**
1. Create: `distant fs write /tmp/orig.txt "original content"`
2. Run `distant fs copy /tmp/orig.txt /tmp/copy.txt`
3. Verify: `distant fs read /tmp/copy.txt`
**Expected Output:** Copy contains "original content".
**Verification:** Content matches original.
**Cleanup:** `distant fs remove /tmp/orig.txt && distant fs remove /tmp/copy.txt`

#### TC-FSC-02: Copy a remote directory

**Category:** Happy Path
**Prerequisites:** Active connection. Remote directory with files.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/dir_orig`
   `distant fs write /tmp/dir_orig/file.txt "data"`
2. Run `distant fs copy /tmp/dir_orig /tmp/dir_copy`
3. Verify: `distant fs read /tmp/dir_copy/file.txt`
**Expected Output:** Copied directory contains the file with matching content.
**Verification:** Content is "data".
**Cleanup:** `distant fs remove --force /tmp/dir_orig && distant fs remove --force /tmp/dir_copy`

#### TC-FSC-03: Copy to existing destination

**Category:** Edge Case
**Prerequisites:** Active connection. Source and destination both exist.
**Steps:**
1. Setup:
   `distant fs write /tmp/src.txt "source"`
   `distant fs write /tmp/dst.txt "old dest"`
2. Run `distant fs copy /tmp/src.txt /tmp/dst.txt`
**Expected Output:** Destination overwritten (or error if overwrite not allowed).
**Verification:** Check content of `/tmp/dst.txt`.
**Cleanup:** `distant fs remove /tmp/src.txt && distant fs remove /tmp/dst.txt`

#### TC-FSC-04: Copy nonexistent source

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs copy /tmp/nonexistent_abc /tmp/dest_abc`
**Expected Output:** Error about source not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSC-05: Copy to non-writable destination path

**Category:** Error Handling
**Prerequisites:** Active connection. A path where the user has no write access.
**Steps:**
1. Create: `distant fs write /tmp/src_perm.txt "test"`
2. Run `distant fs copy /tmp/src_perm.txt /root/dest_perm.txt` (as non-root)
**Expected Output:** Error about permission denied.
**Verification:** Exit code is non-zero.
**Cleanup:** `distant fs remove /tmp/src_perm.txt`

---

## Command: `distant fs exists`

### Purpose

Check whether a specified path exists on the remote machine.

### Dependencies

None beyond the active connection.

### Test Harness Equivalents

None needed — output is self-verifying.

### Test Cases

#### TC-FSE-01: Check existing file

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/exists_test.txt "hi"`
2. Run `distant fs exists /tmp/exists_test.txt`
**Expected Output:** `true` or affirmative indicator.
**Verification:** Output indicates the path exists.
**Cleanup:** `distant fs remove /tmp/exists_test.txt`

#### TC-FSE-02: Check existing directory

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Create: `distant fs make-dir /tmp/exists_dir`
2. Run `distant fs exists /tmp/exists_dir`
**Expected Output:** `true`
**Verification:** Output indicates the path exists.
**Cleanup:** `distant fs remove /tmp/exists_dir`

#### TC-FSE-03: Check nonexistent path

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs exists /tmp/does_not_exist_xyz`
**Expected Output:** `false` or negative indicator.
**Verification:** Output indicates the path does not exist.
**Cleanup:** None

#### TC-FSE-04: Check symlink

**Category:** Edge Case
**Prerequisites:** Active connection. Unix platform. Symlink exists.
**Steps:**
1. Create target: `distant fs write /tmp/link_target.txt "target"`
2. Create symlink on remote (via spawn): `distant spawn -- ln -s /tmp/link_target.txt /tmp/test_link`
3. Run `distant fs exists /tmp/test_link`
**Expected Output:** `true`
**Verification:** Symlink itself exists.
**Cleanup:** `distant fs remove /tmp/test_link && distant fs remove /tmp/link_target.txt`

#### TC-FSE-05: Check broken symlink

**Category:** Edge Case
**Prerequisites:** Active connection. Unix platform.
**Steps:**
1. Create symlink to nonexistent target:
   `distant spawn -- ln -s /tmp/no_target_xyz /tmp/broken_link`
2. Run `distant fs exists /tmp/broken_link`
**Expected Output:** May return `true` (symlink exists) or `false` (target
doesn't exist). Document actual behavior.
**Verification:** Note behavior for broken symlinks.
**Cleanup:** `distant fs remove /tmp/broken_link`

---

## Command: `distant fs make-dir`

### Purpose

Create a directory on the remote machine.

### Dependencies

None beyond the active connection.

### Test Harness Equivalents

None needed — verified via `fs exists` and `fs read`.

### Test Cases

#### TC-FSD-01: Create a single directory

**Category:** Happy Path
**Prerequisites:** Active connection. Parent directory exists.
**Steps:**
1. Run `distant fs make-dir /tmp/new_dir_test`
2. Verify: `distant fs exists /tmp/new_dir_test`
**Expected Output:** No error. Directory created.
**Verification:** `fs exists` returns true. `fs metadata` shows it as a
directory.
**Cleanup:** `distant fs remove /tmp/new_dir_test`

#### TC-FSD-02: Create nested directories with --all

**Category:** Happy Path
**Prerequisites:** Active connection. Intermediate directories do not exist.
**Steps:**
1. Run `distant fs make-dir --all /tmp/parent/child/grandchild`
2. Verify: `distant fs exists /tmp/parent/child/grandchild`
**Expected Output:** All intermediate directories created.
**Verification:** All three levels exist.
**Cleanup:** `distant fs remove --force /tmp/parent`

#### TC-FSD-03: Create directory without --all when parent missing

**Category:** Error Handling
**Prerequisites:** Active connection. Parent does not exist.
**Steps:**
1. Run `distant fs make-dir /tmp/missing_parent/child`
**Expected Output:** Error about parent directory not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSD-04: Create directory that already exists

**Category:** Edge Case
**Prerequisites:** Active connection. Directory already exists.
**Steps:**
1. Create: `distant fs make-dir /tmp/already_exists_dir`
2. Run `distant fs make-dir /tmp/already_exists_dir`
**Expected Output:** Error or no-op (document actual behavior).
**Verification:** Directory still exists. Note whether error or silent success.
**Cleanup:** `distant fs remove /tmp/already_exists_dir`

#### TC-FSD-05: Create directory in non-writable location

**Category:** Error Handling
**Prerequisites:** Active connection. Non-writable parent.
**Steps:**
1. Run `distant fs make-dir /root/no_write_dir` (as non-root)
**Expected Output:** Permission denied error.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant fs metadata`

### Purpose

Retrieve metadata (type, size, permissions, timestamps) for a file, directory,
or symlink on the remote machine.

### Dependencies

- `stat` — cross-reference metadata values

### Test Harness Equivalents

- **`test-file-stat`** — check file metadata

### Test Cases

#### TC-FSM-01: Metadata of a regular file

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/meta_file.txt "hello"`
2. Run `distant fs metadata /tmp/meta_file.txt`
**Expected Output:** Shows file type (file), size (5-6 bytes), permissions, and
timestamps (created, modified, accessed).
**Verification:** File type is "file". Size is reasonable.
**Cleanup:** `distant fs remove /tmp/meta_file.txt`

#### TC-FSM-02: Metadata of a directory

**Category:** Happy Path
**Prerequisites:** Active connection. Directory exists.
**Steps:**
1. Create: `distant fs make-dir /tmp/meta_dir`
2. Run `distant fs metadata /tmp/meta_dir`
**Expected Output:** Shows file type (dir), permissions, timestamps.
**Verification:** Type is "dir".
**Cleanup:** `distant fs remove /tmp/meta_dir`

#### TC-FSM-03: Metadata of a symlink (without resolve)

**Category:** Happy Path
**Prerequisites:** Active connection. Unix. Symlink exists.
**Steps:**
1. Setup:
   `distant fs write /tmp/meta_target.txt "target"`
   `distant spawn -- ln -s /tmp/meta_target.txt /tmp/meta_link`
2. Run `distant fs metadata /tmp/meta_link`
**Expected Output:** Shows file type as symlink.
**Verification:** Type is "symlink".
**Cleanup:** `distant fs remove /tmp/meta_link && distant fs remove /tmp/meta_target.txt`

#### TC-FSM-04: Metadata with --resolve-file-type

**Category:** Happy Path
**Prerequisites:** Active connection. Unix. Symlink to a file.
**Steps:**
1. Setup same as TC-FSM-03
2. Run `distant fs metadata --resolve-file-type /tmp/meta_link`
**Expected Output:** Shows file type as "file" (resolved through symlink).
**Verification:** Type is "file" (not "symlink").
**Cleanup:** Same as TC-FSM-03.

#### TC-FSM-05: Metadata with --canonicalize

**Category:** Happy Path
**Prerequisites:** Active connection. File or symlink exists.
**Steps:**
1. Setup same as TC-FSM-03
2. Run `distant fs metadata --canonicalize /tmp/meta_link`
**Expected Output:** Includes canonicalized path that resolves symlinks and
normalizes components.
**Verification:** Canonicalized path is an absolute path to the actual file.
**Cleanup:** Same as TC-FSM-03.

#### TC-FSM-06: Metadata with both --canonicalize and --resolve-file-type

**Category:** Happy Path
**Prerequisites:** Active connection. Symlink exists.
**Steps:**
1. Setup same as TC-FSM-03
2. Run `distant fs metadata --canonicalize --resolve-file-type /tmp/meta_link`
**Expected Output:** Canonicalized path plus resolved file type.
**Verification:** Both features applied.
**Cleanup:** Same as TC-FSM-03.

#### TC-FSM-07: Metadata of nonexistent path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs metadata /tmp/does_not_exist_meta`
**Expected Output:** Error about path not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant fs read`

### Purpose

Read the contents of a file, or list the entries within a directory on the
remote machine.

### Dependencies

- `cat` — cross-reference file contents
- `ls` — cross-reference directory listings

### Test Harness Equivalents

- **`test-file-read`** — read and print file contents

### Test Cases

#### TC-FSR-01: Read a text file

**Category:** Happy Path
**Prerequisites:** Active connection. Remote file exists.
**Steps:**
1. Create: `distant fs write /tmp/read_test.txt "hello world"`
2. Run `distant fs read /tmp/read_test.txt`
**Expected Output:** `hello world`
**Verification:** Output matches written content.
**Cleanup:** `distant fs remove /tmp/read_test.txt`

#### TC-FSR-02: Read an empty file

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Create: `distant fs write /tmp/empty_file.txt ""`
2. Run `distant fs read /tmp/empty_file.txt`
**Expected Output:** Empty output (no bytes).
**Verification:** Output is empty or contains no printable characters.
**Cleanup:** `distant fs remove /tmp/empty_file.txt`

#### TC-FSR-03: Read a directory (default depth=1)

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with known files.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/read_dir`
   `distant fs write /tmp/read_dir/a.txt "a"`
   `distant fs write /tmp/read_dir/b.txt "b"`
2. Run `distant fs read /tmp/read_dir`
**Expected Output:** Lists `a.txt` and `b.txt` (relative paths by default).
**Verification:** Both files listed.
**Cleanup:** `distant fs remove --force /tmp/read_dir`

#### TC-FSR-04: Read directory with --depth 0 (unlimited)

**Category:** Happy Path
**Prerequisites:** Active connection. Nested directory structure.
**Steps:**
1. Setup:
   `distant fs make-dir --all /tmp/deep_dir/sub/nested`
   `distant fs write /tmp/deep_dir/top.txt "top"`
   `distant fs write /tmp/deep_dir/sub/mid.txt "mid"`
   `distant fs write /tmp/deep_dir/sub/nested/bot.txt "bot"`
2. Run `distant fs read --depth 0 /tmp/deep_dir`
**Expected Output:** Lists all files at all levels: `top.txt`, `sub/`,
`sub/mid.txt`, `sub/nested/`, `sub/nested/bot.txt`.
**Verification:** All nested entries appear.
**Cleanup:** `distant fs remove --force /tmp/deep_dir`

#### TC-FSR-05: Read directory with --depth 1 (immediate children only)

**Category:** Happy Path
**Prerequisites:** Same as TC-FSR-04.
**Steps:**
1. Same setup as TC-FSR-04.
2. Run `distant fs read --depth 1 /tmp/deep_dir`
**Expected Output:** Lists only `top.txt` and `sub/` (not nested contents).
**Verification:** Only immediate children listed.
**Cleanup:** Same as TC-FSR-04.

#### TC-FSR-06: Read directory with --absolute

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with files.
**Steps:**
1. Same setup as TC-FSR-03.
2. Run `distant fs read --absolute /tmp/read_dir`
**Expected Output:** Lists absolute paths: `/tmp/read_dir/a.txt`,
`/tmp/read_dir/b.txt`.
**Verification:** All paths start with `/`.
**Cleanup:** Same as TC-FSR-03.

#### TC-FSR-07: Read directory with --canonicalize

**Category:** Happy Path
**Prerequisites:** Active connection. Directory containing a symlink.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/canon_dir`
   `distant fs write /tmp/canon_dir/real.txt "data"`
   `distant spawn -- ln -s /tmp/canon_dir/real.txt /tmp/canon_dir/link.txt`
2. Run `distant fs read --absolute --canonicalize /tmp/canon_dir`
**Expected Output:** Paths are canonicalized (symlinks resolved).
**Verification:** Link entry resolves to the real file path.
**Cleanup:** `distant fs remove --force /tmp/canon_dir`

#### TC-FSR-08: Read directory with --include-root

**Category:** Happy Path
**Prerequisites:** Active connection. Directory exists.
**Steps:**
1. Same setup as TC-FSR-03.
2. Run `distant fs read --include-root /tmp/read_dir`
**Expected Output:** Listing includes the root directory itself as the first
entry, in addition to its contents.
**Verification:** Root directory path appears in output.
**Cleanup:** Same as TC-FSR-03.

#### TC-FSR-09: Read nonexistent path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs read /tmp/no_such_file_xyz`
**Expected Output:** Error about path not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSR-10: Read a binary file

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Create a file with binary content via spawn:
   `distant spawn -- sh -c 'printf "\\x00\\x01\\x02\\xff" > /tmp/binary_test.bin'`
2. Run `distant fs read /tmp/binary_test.bin`
**Expected Output:** Binary bytes output to stdout (may be garbled in terminal
but data is present).
**Verification:** Pipe output to `wc -c` — should be 4 bytes.
**Cleanup:** `distant fs remove /tmp/binary_test.bin`

---

## Command: `distant fs remove`

### Purpose

Remove a file or directory on the remote machine.

### Dependencies

None beyond the active connection.

### Test Harness Equivalents

None needed — verified via `fs exists`.

### Test Cases

#### TC-FSRM-01: Remove a file

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/rm_file.txt "delete me"`
2. Run `distant fs remove /tmp/rm_file.txt`
3. Verify: `distant fs exists /tmp/rm_file.txt`
**Expected Output:** No error. File removed.
**Verification:** `fs exists` returns false.
**Cleanup:** None

#### TC-FSRM-02: Remove an empty directory

**Category:** Happy Path
**Prerequisites:** Active connection. Empty directory exists.
**Steps:**
1. Create: `distant fs make-dir /tmp/rm_empty_dir`
2. Run `distant fs remove /tmp/rm_empty_dir`
**Expected Output:** No error. Directory removed.
**Verification:** `fs exists` returns false.
**Cleanup:** None

#### TC-FSRM-03: Remove a non-empty directory without --force

**Category:** Error Handling
**Prerequisites:** Active connection. Directory with contents.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/rm_full_dir`
   `distant fs write /tmp/rm_full_dir/file.txt "data"`
2. Run `distant fs remove /tmp/rm_full_dir`
**Expected Output:** Error about directory not empty.
**Verification:** Exit code is non-zero. Directory still exists.
**Cleanup:** `distant fs remove --force /tmp/rm_full_dir`

#### TC-FSRM-04: Remove a non-empty directory with --force

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with contents.
**Steps:**
1. Setup same as TC-FSRM-03
2. Run `distant fs remove --force /tmp/rm_full_dir`
**Expected Output:** No error. Directory and all contents removed.
**Verification:** `fs exists` returns false.
**Cleanup:** None

#### TC-FSRM-05: Remove nonexistent path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs remove /tmp/does_not_exist_rm`
**Expected Output:** Error about path not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSRM-06: Remove a symlink

**Category:** Edge Case
**Prerequisites:** Active connection. Unix. Symlink exists.
**Steps:**
1. Setup:
   `distant fs write /tmp/rm_link_target.txt "target"`
   `distant spawn -- ln -s /tmp/rm_link_target.txt /tmp/rm_link`
2. Run `distant fs remove /tmp/rm_link`
3. Verify: `distant fs exists /tmp/rm_link` and
   `distant fs exists /tmp/rm_link_target.txt`
**Expected Output:** Symlink removed. Target file still exists.
**Verification:** Link gone, target still there.
**Cleanup:** `distant fs remove /tmp/rm_link_target.txt`

#### TC-FSRM-07: Remove file in non-writable directory

**Category:** Error Handling
**Prerequisites:** Active connection. File in a non-writable directory.
**Steps:**
1. Run `distant fs remove /root/some_file` (as non-root)
**Expected Output:** Permission denied error.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant fs rename`

### Purpose

Move or rename a file or directory on the remote machine.

### Dependencies

None beyond the active connection.

### Test Harness Equivalents

None needed — verified via `fs exists` and `fs read`.

### Test Cases

#### TC-FSRN-01: Rename a file

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/rename_src.txt "content"`
2. Run `distant fs rename /tmp/rename_src.txt /tmp/rename_dst.txt`
3. Verify: `distant fs exists /tmp/rename_src.txt` and
   `distant fs read /tmp/rename_dst.txt`
**Expected Output:** Source gone. Destination has original content.
**Verification:** Source doesn't exist. Destination contains "content".
**Cleanup:** `distant fs remove /tmp/rename_dst.txt`

#### TC-FSRN-02: Rename a directory

**Category:** Happy Path
**Prerequisites:** Active connection. Directory exists.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/rename_dir_src`
   `distant fs write /tmp/rename_dir_src/file.txt "data"`
2. Run `distant fs rename /tmp/rename_dir_src /tmp/rename_dir_dst`
3. Verify: `distant fs read /tmp/rename_dir_dst/file.txt`
**Expected Output:** Directory moved with contents intact.
**Verification:** Contents accessible at new path.
**Cleanup:** `distant fs remove --force /tmp/rename_dir_dst`

#### TC-FSRN-03: Rename to an existing path

**Category:** Edge Case
**Prerequisites:** Active connection. Both source and destination exist.
**Steps:**
1. Setup:
   `distant fs write /tmp/rn_src.txt "new"`
   `distant fs write /tmp/rn_dst.txt "old"`
2. Run `distant fs rename /tmp/rn_src.txt /tmp/rn_dst.txt`
**Expected Output:** On most platforms, destination is overwritten. Source gone.
**Verification:** Destination contains "new". Source doesn't exist.
**Cleanup:** `distant fs remove /tmp/rn_dst.txt`

#### TC-FSRN-04: Rename nonexistent source

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs rename /tmp/no_exist_rn /tmp/rn_dest`
**Expected Output:** Error about source not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSRN-05: Rename to non-writable location

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Create: `distant fs write /tmp/rn_perm.txt "test"`
2. Run `distant fs rename /tmp/rn_perm.txt /root/rn_perm.txt` (as non-root)
**Expected Output:** Permission denied error.
**Verification:** Exit code is non-zero. Source still exists.
**Cleanup:** `distant fs remove /tmp/rn_perm.txt`

---

## Command: `distant fs search`

### Purpose

Search files and directories on the remote machine by content or path patterns,
with support for gitignore integration, depth limiting, and pagination.

### Dependencies

- Known test directory with files of known content for searching

### Test Harness Equivalents

None needed — output is self-verifying.

### Test Cases

#### TC-FSS-01: Search by file contents (default)

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with searchable files.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/search_dir`
   `distant fs write /tmp/search_dir/a.txt "hello world"`
   `distant fs write /tmp/search_dir/b.txt "goodbye world"`
   `distant fs write /tmp/search_dir/c.txt "no match here"`
2. Run `distant fs search "hello" /tmp/search_dir`
**Expected Output:** Match found in `a.txt` showing the line "hello world".
**Verification:** Only `a.txt` appears in results.
**Cleanup:** `distant fs remove --force /tmp/search_dir`

#### TC-FSS-02: Search by path

**Category:** Happy Path
**Prerequisites:** Same setup as TC-FSS-01.
**Steps:**
1. Run `distant fs search --target path "a\\.txt" /tmp/search_dir`
**Expected Output:** `a.txt` listed as matching path.
**Verification:** Only `a.txt` appears.
**Cleanup:** Same as TC-FSS-01.

#### TC-FSS-03: Search with regex pattern

**Category:** Happy Path
**Prerequisites:** Same setup as TC-FSS-01.
**Steps:**
1. Run `distant fs search "he..o" /tmp/search_dir`
**Expected Output:** Match in `a.txt` (regex `he..o` matches "hello").
**Verification:** `a.txt` matched.
**Cleanup:** Same as TC-FSS-01.

#### TC-FSS-04: Search with --include filter

**Category:** Happy Path
**Prerequisites:** Same setup as TC-FSS-01.
**Steps:**
1. Run `distant fs search --include "a\\.txt" "world" /tmp/search_dir`
**Expected Output:** Only `a.txt` searched and matched (even though `b.txt`
also contains "world").
**Verification:** Only `a.txt` in results.
**Cleanup:** Same as TC-FSS-01.

#### TC-FSS-05: Search with --exclude filter

**Category:** Happy Path
**Prerequisites:** Same setup as TC-FSS-01.
**Steps:**
1. Run `distant fs search --exclude "a\\.txt" "world" /tmp/search_dir`
**Expected Output:** `b.txt` matched but not `a.txt`.
**Verification:** `a.txt` excluded from results.
**Cleanup:** Same as TC-FSS-01.

#### TC-FSS-06: Search with --max-depth

**Category:** Happy Path
**Prerequisites:** Active connection. Nested directory structure.
**Steps:**
1. Setup:
   `distant fs make-dir --all /tmp/search_deep/sub`
   `distant fs write /tmp/search_deep/top.txt "match"`
   `distant fs write /tmp/search_deep/sub/deep.txt "match"`
2. Run `distant fs search --max-depth 1 "match" /tmp/search_deep`
**Expected Output:** Only `top.txt` matched (depth 1 = immediate children only).
**Verification:** `deep.txt` not in results.
**Cleanup:** `distant fs remove --force /tmp/search_deep`

#### TC-FSS-07: Search with --upward

**Category:** Happy Path
**Prerequisites:** Active connection. Nested directory.
**Steps:**
1. Setup:
   `distant fs make-dir --all /tmp/up_search/child`
   `distant fs write /tmp/up_search/marker.txt "found"`
2. Run `distant fs search --upward "found" /tmp/up_search/child`
**Expected Output:** Searches upward from `/tmp/up_search/child` through parent
directories. Finds `marker.txt` in parent.
**Verification:** Match found in parent directory.
**Cleanup:** `distant fs remove --force /tmp/up_search`

#### TC-FSS-08: Search with --limit

**Category:** Happy Path
**Prerequisites:** Active connection. Multiple matching files.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/limit_search`
   `distant fs write /tmp/limit_search/1.txt "match"`
   `distant fs write /tmp/limit_search/2.txt "match"`
   `distant fs write /tmp/limit_search/3.txt "match"`
2. Run `distant fs search --limit 1 "match" /tmp/limit_search`
**Expected Output:** Only 1 result returned.
**Verification:** Exactly one match in output.
**Cleanup:** `distant fs remove --force /tmp/limit_search`

#### TC-FSS-09: Search with --pagination

**Category:** Happy Path
**Prerequisites:** Active connection. Multiple matching files.
**Steps:**
1. Same setup as TC-FSS-08 (3 files).
2. Run `distant fs search --pagination 1 "match" /tmp/limit_search`
**Expected Output:** Results delivered in batches of 1 (streaming behavior).
**Verification:** All 3 matches eventually appear.
**Cleanup:** Same as TC-FSS-08.

#### TC-FSS-10: Search with --use-git-ignore-files

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with `.gitignore`.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/git_search`
   `distant fs write /tmp/git_search/.gitignore "ignored.txt"`
   `distant fs write /tmp/git_search/kept.txt "match"`
   `distant fs write /tmp/git_search/ignored.txt "match"`
2. Run `distant fs search --use-git-ignore-files "match" /tmp/git_search`
**Expected Output:** Only `kept.txt` matched. `ignored.txt` is excluded by
`.gitignore`.
**Verification:** `ignored.txt` not in results.
**Cleanup:** `distant fs remove --force /tmp/git_search`

#### TC-FSS-11: Search with --ignore-hidden

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/hidden_search`
   `distant fs write /tmp/hidden_search/.hidden "match"`
   `distant fs write /tmp/hidden_search/visible.txt "match"`
2. Run `distant fs search --ignore-hidden "match" /tmp/hidden_search`
**Expected Output:** Only `visible.txt` matched.
**Verification:** `.hidden` excluded.
**Cleanup:** `distant fs remove --force /tmp/hidden_search`

#### TC-FSS-12: Search with --follow-symbolic-links

**Category:** Edge Case
**Prerequisites:** Active connection. Unix. Symlink to another directory.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/search_link_target`
   `distant fs write /tmp/search_link_target/file.txt "findme"`
   `distant fs make-dir /tmp/search_link_dir`
   `distant spawn -- ln -s /tmp/search_link_target /tmp/search_link_dir/link`
2. Run `distant fs search --follow-symbolic-links "findme" /tmp/search_link_dir`
**Expected Output:** Match found in the symlinked directory.
**Verification:** File through symlink is found.
**Cleanup:** `distant fs remove --force /tmp/search_link_dir && distant fs remove --force /tmp/search_link_target`

#### TC-FSS-13: Search with no matches

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Setup: `distant fs make-dir /tmp/no_match_dir && distant fs write /tmp/no_match_dir/file.txt "abc"`
2. Run `distant fs search "xyz_no_match" /tmp/no_match_dir`
**Expected Output:** No matches found. Empty result or informational message.
**Verification:** No results in output. Exit code may still be 0.
**Cleanup:** `distant fs remove --force /tmp/no_match_dir`

#### TC-FSS-14: Search with --use-ignore-files

**Category:** Happy Path
**Prerequisites:** Active connection. Directory with `.ignore` file.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/ignore_search`
   `distant fs write /tmp/ignore_search/.ignore "skip.txt"`
   `distant fs write /tmp/ignore_search/keep.txt "match"`
   `distant fs write /tmp/ignore_search/skip.txt "match"`
2. Run `distant fs search --use-ignore-files "match" /tmp/ignore_search`
**Expected Output:** Only `keep.txt` matched.
**Verification:** `skip.txt` excluded.
**Cleanup:** `distant fs remove --force /tmp/ignore_search`

---

## Command: `distant fs set-permissions`

### Purpose

Set permissions for a file, directory, or symlink on the remote machine.
Supports numeric mode (e.g., `755`), symbolic mode (e.g., `u+x`), and special
keywords `readonly` and `notreadonly`.

### Dependencies

- `stat` — verify permissions after setting

### Test Harness Equivalents

- **`test-file-stat`** — check file permissions

### Test Cases

#### TC-FSP-01: Set numeric permissions (Unix)

**Category:** Happy Path
**Prerequisites:** Active connection. Unix platform. File exists.
**Steps:**
1. Create: `distant fs write /tmp/perm_test.txt "data"`
2. Run `distant fs set-permissions 644 /tmp/perm_test.txt`
3. Verify: `distant fs metadata /tmp/perm_test.txt`
**Expected Output:** Permissions set to `rw-r--r--` (644).
**Verification:** Metadata shows 644 permissions.
**Cleanup:** `distant fs remove /tmp/perm_test.txt`

#### TC-FSP-02: Set executable permission (symbolic, Unix)

**Category:** Happy Path
**Prerequisites:** Active connection. Unix platform.
**Steps:**
1. Create: `distant fs write /tmp/exec_test.sh "#!/bin/sh\necho hi"`
2. Run `distant fs set-permissions u+x /tmp/exec_test.sh`
3. Verify: `distant fs metadata /tmp/exec_test.sh`
**Expected Output:** Owner execute bit set.
**Verification:** Metadata shows execute permission for owner.
**Cleanup:** `distant fs remove /tmp/exec_test.sh`

#### TC-FSP-03: Set readonly

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Create: `distant fs write /tmp/readonly_test.txt "data"`
2. Run `distant fs set-permissions readonly /tmp/readonly_test.txt`
3. Verify: `distant fs metadata /tmp/readonly_test.txt`
**Expected Output:** File is now read-only.
**Verification:** Metadata shows readonly. Attempting to write fails.
**Cleanup:** `distant fs set-permissions notreadonly /tmp/readonly_test.txt && distant fs remove /tmp/readonly_test.txt`

#### TC-FSP-04: Set notreadonly

**Category:** Happy Path
**Prerequisites:** Active connection. File is currently readonly.
**Steps:**
1. Create readonly file: `distant fs write /tmp/rw_test.txt "data"` then
   `distant fs set-permissions readonly /tmp/rw_test.txt`
2. Run `distant fs set-permissions notreadonly /tmp/rw_test.txt`
3. Verify: `distant fs write /tmp/rw_test.txt "updated"` succeeds
**Expected Output:** File is now writable again.
**Verification:** Write succeeds.
**Cleanup:** `distant fs remove /tmp/rw_test.txt`

#### TC-FSP-05: Set permissions recursively

**Category:** Happy Path
**Prerequisites:** Active connection. Unix. Directory with files.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/perm_recurse`
   `distant fs write /tmp/perm_recurse/a.txt "a"`
   `distant fs write /tmp/perm_recurse/b.txt "b"`
2. Run `distant fs set-permissions -R 600 /tmp/perm_recurse`
3. Verify: `distant fs metadata /tmp/perm_recurse/a.txt`
**Expected Output:** All files and the directory have 600 permissions.
**Verification:** Metadata shows 600 for files.
**Cleanup:** `distant fs remove --force /tmp/perm_recurse`

#### TC-FSP-06: Set permissions with --follow-symlinks

**Category:** Edge Case
**Prerequisites:** Active connection. Unix. Symlink exists.
**Steps:**
1. Setup:
   `distant fs write /tmp/perm_sym_target.txt "data"`
   `distant spawn -- ln -s /tmp/perm_sym_target.txt /tmp/perm_sym_link`
2. Run `distant fs set-permissions -L 444 /tmp/perm_sym_link`
**Expected Output:** The symlink itself is unaffected; target's permissions
changed to 444 (as per help: "Follow symlinks, which means that they will be
unaffected").
**Verification:** Target file has 444 permissions.
**Cleanup:** `distant fs remove /tmp/perm_sym_link && distant fs remove /tmp/perm_sym_target.txt`

#### TC-FSP-07: Set permissions on nonexistent path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs set-permissions 644 /tmp/no_such_perm_file`
**Expected Output:** Error about path not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSP-08: Set invalid mode string

**Category:** Error Handling
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/bad_mode.txt "data"`
2. Run `distant fs set-permissions zzz /tmp/bad_mode.txt`
**Expected Output:** Error about invalid permission mode.
**Verification:** Exit code is non-zero.
**Cleanup:** `distant fs remove /tmp/bad_mode.txt`

#### TC-FSP-09: Set permissions 000 (no access, Unix)

**Category:** Boundary
**Prerequisites:** Active connection. Unix.
**Steps:**
1. Create: `distant fs write /tmp/no_access.txt "data"`
2. Run `distant fs set-permissions 000 /tmp/no_access.txt`
3. Verify: `distant fs read /tmp/no_access.txt` (may fail if running as
non-root)
**Expected Output:** Permissions set to 000.
**Verification:** `fs metadata` shows 000. Read attempt fails (if not root).
**Cleanup:** `distant fs set-permissions 644 /tmp/no_access.txt && distant fs remove /tmp/no_access.txt`

#### TC-FSP-10: Set permissions 777 (full access, Unix)

**Category:** Boundary
**Prerequisites:** Active connection. Unix.
**Steps:**
1. Create: `distant fs write /tmp/full_access.txt "data"`
2. Run `distant fs set-permissions 777 /tmp/full_access.txt`
3. Verify: `distant fs metadata /tmp/full_access.txt`
**Expected Output:** Permissions set to 777.
**Verification:** Metadata shows `rwxrwxrwx`.
**Cleanup:** `distant fs remove /tmp/full_access.txt`

---

## Command: `distant fs watch`

### Purpose

Watch a path for changes on the remote machine. Outputs events as they occur
(long-running command).

### Dependencies

- A second terminal/process to make changes while watching
- `touch` / `echo` — create file changes to trigger events

### Test Harness Equivalents

- **`test-file-read`** — write files to trigger events
- **`test-sleep`** — wait for events to be reported

### Test Cases

#### TC-FSW-01: Watch a file for modification

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/watch_file.txt "initial"`
2. In terminal A: `distant fs watch /tmp/watch_file.txt`
3. In terminal B: `distant fs write /tmp/watch_file.txt "updated"`
4. Observe output in terminal A
**Expected Output:** Terminal A shows a change event (modify or close_write)
for the file.
**Verification:** Event appears in watch output.
**Cleanup:** Ctrl+C terminal A. `distant fs remove /tmp/watch_file.txt`

#### TC-FSW-02: Watch a directory (non-recursive)

**Category:** Happy Path
**Prerequisites:** Active connection. Directory exists.
**Steps:**
1. Create: `distant fs make-dir /tmp/watch_dir`
2. In terminal A: `distant fs watch /tmp/watch_dir`
3. In terminal B: `distant fs write /tmp/watch_dir/new_file.txt "data"`
4. Observe terminal A
**Expected Output:** Create event for `new_file.txt`.
**Verification:** Event mentions the new file.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_dir`

#### TC-FSW-03: Watch a directory recursively

**Category:** Happy Path
**Prerequisites:** Active connection. Nested directory.
**Steps:**
1. Setup:
   `distant fs make-dir --all /tmp/watch_rec/sub`
2. In terminal A: `distant fs watch --recursive /tmp/watch_rec`
3. In terminal B: `distant fs write /tmp/watch_rec/sub/deep.txt "data"`
4. Observe terminal A
**Expected Output:** Event for the file in the subdirectory.
**Verification:** Event captures change in nested directory.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_rec`

#### TC-FSW-04: Watch with --only create filter

**Category:** Happy Path
**Prerequisites:** Active connection. Directory exists.
**Steps:**
1. Create: `distant fs make-dir /tmp/watch_only`
2. In terminal A: `distant fs watch --only create /tmp/watch_only`
3. In terminal B: Create and then modify a file:
   `distant fs write /tmp/watch_only/f.txt "new"`
   `distant fs write /tmp/watch_only/f.txt "changed"`
4. Observe terminal A
**Expected Output:** Only the create event appears. Modification event is
filtered out.
**Verification:** No modify/close_write events in output.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_only`

#### TC-FSW-05: Watch with --only delete filter

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/watch_del`
   `distant fs write /tmp/watch_del/to_delete.txt "bye"`
2. In terminal A: `distant fs watch --only delete /tmp/watch_del`
3. In terminal B: `distant fs remove /tmp/watch_del/to_delete.txt`
4. Observe terminal A
**Expected Output:** Delete event for `to_delete.txt`.
**Verification:** Only delete event captured.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_del`

#### TC-FSW-06: Watch with --except modify filter

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Setup: `distant fs make-dir /tmp/watch_except`
2. In terminal A: `distant fs watch --except modify /tmp/watch_except`
3. In terminal B: `distant fs write /tmp/watch_except/f.txt "new"` (creates)
   then `distant fs write /tmp/watch_except/f.txt "mod"` (modifies)
4. Observe terminal A
**Expected Output:** Create event appears, modify events filtered out.
**Verification:** No modify events in output.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_except`

#### TC-FSW-07: Watch with --only rename filter

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Setup:
   `distant fs make-dir /tmp/watch_rename`
   `distant fs write /tmp/watch_rename/old.txt "data"`
2. In terminal A: `distant fs watch --only rename /tmp/watch_rename`
3. In terminal B: `distant fs rename /tmp/watch_rename/old.txt /tmp/watch_rename/new.txt`
4. Observe terminal A
**Expected Output:** Rename event captured.
**Verification:** Event reflects the rename operation.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_rename`

#### TC-FSW-08: Watch with multiple --only filters

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Setup: `distant fs make-dir /tmp/watch_multi`
2. In terminal A: `distant fs watch --only create --only delete /tmp/watch_multi`
3. In terminal B: Create and delete a file:
   `distant fs write /tmp/watch_multi/temp.txt "data"`
   `distant fs remove /tmp/watch_multi/temp.txt`
4. Observe terminal A
**Expected Output:** Both create and delete events appear. No modify events.
**Verification:** Only specified event types in output.
**Cleanup:** Ctrl+C. `distant fs remove --force /tmp/watch_multi`

#### TC-FSW-09: Watch nonexistent path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs watch /tmp/does_not_exist_watch`
**Expected Output:** Error about path not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant fs write`

### Purpose

Write contents to a file on the remote machine. Content can come from an
argument or stdin. Supports overwrite and append modes.

### Dependencies

- `cat` — verify file contents locally
- `diff` — compare written content

### Test Harness Equivalents

- **`test-file-read`** — read and verify contents
- **`test-file-diff`** — compare files

### Test Cases

#### TC-FSWR-01: Write new file with inline data

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs write /tmp/write_new.txt "hello world"`
2. Verify: `distant fs read /tmp/write_new.txt`
**Expected Output:** File created with content "hello world".
**Verification:** Read returns "hello world".
**Cleanup:** `distant fs remove /tmp/write_new.txt`

#### TC-FSWR-02: Overwrite existing file

**Category:** Happy Path
**Prerequisites:** Active connection. File exists.
**Steps:**
1. Create: `distant fs write /tmp/write_over.txt "original"`
2. Run `distant fs write /tmp/write_over.txt "replaced"`
3. Verify: `distant fs read /tmp/write_over.txt`
**Expected Output:** File now contains "replaced" (not "original").
**Verification:** Content is "replaced".
**Cleanup:** `distant fs remove /tmp/write_over.txt`

#### TC-FSWR-03: Append to existing file

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Create: `distant fs write /tmp/write_append.txt "first"`
2. Run `distant fs write --append /tmp/write_append.txt " second"`
3. Verify: `distant fs read /tmp/write_append.txt`
**Expected Output:** File contains "first second".
**Verification:** Both parts present.
**Cleanup:** `distant fs remove /tmp/write_append.txt`

#### TC-FSWR-04: Write from stdin

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `echo "piped data" | distant fs write /tmp/write_stdin.txt`
2. Verify: `distant fs read /tmp/write_stdin.txt`
**Expected Output:** File contains "piped data".
**Verification:** Content matches piped input.
**Cleanup:** `distant fs remove /tmp/write_stdin.txt`

#### TC-FSWR-05: Write empty content

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs write /tmp/write_empty.txt ""`
2. Verify: `distant fs read /tmp/write_empty.txt`
**Expected Output:** File created but empty.
**Verification:** File exists with 0 bytes.
**Cleanup:** `distant fs remove /tmp/write_empty.txt`

#### TC-FSWR-06: Write to non-writable path

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs write /root/no_write_file.txt "data"` (as non-root)
**Expected Output:** Permission denied error.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSWR-07: Write to path in nonexistent directory

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant fs write /tmp/no_parent_dir_xyz/file.txt "data"`
**Expected Output:** Error about parent directory not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-FSWR-08: Write large content

**Category:** Boundary
**Prerequisites:** Active connection.
**Steps:**
1. Generate large content: `dd if=/dev/urandom bs=1024 count=1024 2>/dev/null | base64 > /tmp/large_local.txt`
2. Run `cat /tmp/large_local.txt | distant fs write /tmp/large_remote.txt`
3. Verify: `distant fs metadata /tmp/large_remote.txt` (check size)
**Expected Output:** Large file written successfully.
**Verification:** File size on remote matches local file size.
**Cleanup:** `distant fs remove /tmp/large_remote.txt && rm /tmp/large_local.txt`

---

### 5D. TCP Tunneling

---

## Command: `distant tunnel open`

### Purpose

Open a forward tunnel: bind a local port and forward TCP connections through the
remote server to a target host and port. The remote server makes the outbound
TCP connection, so the target only needs to be reachable from the remote network.

**Spec format:** `LOCAL_PORT[:REMOTE_HOST]:REMOTE_PORT`
- `8080:3000` — local:8080 → remote localhost:3000
- `8080:db.internal:5432` — local:8080 → db.internal:5432 via remote

### Dependencies

- `nc` — listen on remote-side port to accept tunneled connections; send data
  through tunnel from local side
- `kill` — stop listener processes

### Test Harness Equivalents

- **`test-tcp-peer`** — TCP listener (on remote) and connector (on local) for
  verifying data flows through the forward tunnel

### Test Cases

#### TC-TFO-01: Open forward tunnel (shorthand spec)

**Category:** Happy Path
**Prerequisites:** Active connection. Remote port 9001 has a TCP listener
(e.g., `distant spawn -- nc -l 9001`).
**Steps:**
1. Start a listener on the remote side:
   `distant spawn -- sh -c "echo 'hello from remote' | nc -l 9001"` (background)
2. Run `distant tunnel open 8081:9001`
3. Connect locally: `echo "ping" | nc localhost 8081`
**Expected Output:** Tunnel opens. `nc localhost 8081` receives "hello from
remote" from the remote listener.
**Verification:** Data flows through the tunnel. Tunnel ID is printed.
**Cleanup:** `distant tunnel close <ID>`

#### TC-TFO-02: Open forward tunnel (full spec with host)

**Category:** Happy Path
**Prerequisites:** Active connection. Remote can reach localhost.
**Steps:**
1. Start remote listener: `distant spawn -- sh -c "echo 'data' | nc -l 9002"`
2. Run `distant tunnel open 8082:127.0.0.1:9002`
3. Connect locally: `nc localhost 8082`
**Expected Output:** Same behavior as TC-TFO-01 but with explicit remote host.
**Verification:** Data received through tunnel.
**Cleanup:** `distant tunnel close <ID>`

#### TC-TFO-03: Open forward tunnel to third-party host

**Category:** Happy Path
**Prerequisites:** Active connection. A service reachable from the remote
network (in self-to-self mode, use a local listener).
**Steps:**
1. Start a local listener on port 9003: `nc -l 9003 &`
   (In self-to-self mode, remote=local, so this simulates a "third-party" host)
2. Run `distant tunnel open 8083:localhost:9003`
3. Connect: `echo "test" | nc localhost 8083`
**Expected Output:** Data flows from local:8083 → remote → localhost:9003.
**Verification:** The listener on 9003 receives "test".
**Cleanup:** `distant tunnel close <ID>`. Kill the nc listener.

#### TC-TFO-04: Open forward tunnel on already-bound local port

**Category:** Error Handling
**Prerequisites:** Active connection. Port 8084 already bound locally.
**Steps:**
1. Bind local port: `nc -l 8084 &`
2. Run `distant tunnel open 8084:9000`
**Expected Output:** Error about local port 8084 already in use.
**Verification:** Exit code is non-zero.
**Cleanup:** Kill the nc listener.

#### TC-TFO-05: Open forward tunnel with invalid spec format

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel open not_a_spec`
**Expected Output:** Error about invalid tunnel spec format.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-TFO-06: Open forward tunnel with invalid spec (missing port)

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel open 8080:`
**Expected Output:** Error about invalid spec.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-TFO-07: Open forward tunnel with --connection flag

**Category:** Happy Path
**Prerequisites:** Multiple connections active. Remote listener on one.
**Steps:**
1. Run `distant tunnel open --connection <ID> 8087:9007`
**Expected Output:** Tunnel created on the specified connection.
**Verification:** `distant tunnel list --connection <ID>` shows the tunnel.
**Cleanup:** `distant tunnel close <ID>`

#### TC-TFO-08: Forward tunnel data bidirectional flow

**Category:** Happy Path
**Prerequisites:** Active connection. Remote echo server.
**Steps:**
1. Start remote echo service:
   `distant spawn -- sh -c "nc -l 9008 | while read line; do echo \"echo: $line\"; done"`
2. Run `distant tunnel open 8088:9008`
3. Send data: `echo "hello" | nc localhost 8088`
**Expected Output:** Bidirectional data flows through the tunnel.
**Verification:** Response received from remote echo service.
**Cleanup:** `distant tunnel close <ID>`

#### TC-TFO-09: Open multiple forward tunnels

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel open 8091:9091`
2. Run `distant tunnel open 8092:9092`
3. Run `distant tunnel list`
**Expected Output:** Both tunnels listed as active.
**Verification:** `tunnel list` shows two entries with distinct IDs.
**Cleanup:** Close both tunnels.

#### TC-TFO-10: Open forward tunnel when no connection active

**Category:** Error Handling
**Prerequisites:** Manager running but no active connections.
**Steps:**
1. Run `distant tunnel open 8090:9090`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant tunnel listen`

### Purpose

Open a reverse tunnel: bind a port on the remote server and forward TCP
connections back through the manager to a target host and port on the local
network.

**Spec format:** `REMOTE_PORT[:LOCAL_HOST]:LOCAL_PORT`
- `9090:3000` — remote:9090 → local localhost:3000
- `9090:dev-server:3000` — remote:9090 → dev-server:3000 via local

### Dependencies

- `nc` — listen on local-side port to receive tunneled connections; connect
  from remote to trigger the tunnel
- `kill` — stop processes

### Test Harness Equivalents

- **`test-tcp-peer`** — TCP listener (on local) and connector (on remote) for
  verifying reverse data flow

### Test Cases

#### TC-TRL-01: Open reverse tunnel (shorthand spec)

**Category:** Happy Path
**Prerequisites:** Active connection. Local port 3001 has a TCP listener.
**Steps:**
1. Start local listener: `echo "hello from local" | nc -l 3001 &`
2. Run `distant tunnel listen 9091:3001`
3. Connect from remote: `distant spawn -- nc localhost 9091`
**Expected Output:** Tunnel opens. Remote connection to 9091 receives "hello
from local".
**Verification:** Data flows from remote:9091 → local:3001.
**Cleanup:** `distant tunnel close <ID>`. Kill nc listener.

#### TC-TRL-02: Open reverse tunnel (full spec with local host)

**Category:** Happy Path
**Prerequisites:** Active connection. Local listener on 127.0.0.1:3002.
**Steps:**
1. Start local listener: `echo "data" | nc -l 3002 &`
2. Run `distant tunnel listen 9092:127.0.0.1:3002`
3. Connect from remote: `distant spawn -- nc localhost 9092`
**Expected Output:** Data received through reverse tunnel.
**Verification:** Remote receives "data".
**Cleanup:** `distant tunnel close <ID>`. Kill nc listener.

#### TC-TRL-03: Reverse tunnel to third-party local-network host

**Category:** Happy Path
**Prerequisites:** Active connection. In self-to-self mode, use a second local
listener to simulate a third-party host.
**Steps:**
1. Start listener: `echo "third-party" | nc -l 3003 &`
2. Run `distant tunnel listen 9093:localhost:3003`
3. Connect from remote: `distant spawn -- nc localhost 9093`
**Expected Output:** Data flows remote:9093 → local machine → localhost:3003.
**Verification:** Remote receives "third-party".
**Cleanup:** `distant tunnel close <ID>`. Kill nc listener.

#### TC-TRL-04: Reverse tunnel with remote port conflict

**Category:** Error Handling
**Prerequisites:** Active connection. Remote port 9094 already bound.
**Steps:**
1. Bind remote port: `distant spawn -- nc -l 9094 &`
2. Run `distant tunnel listen 9094:3000`
**Expected Output:** Error about remote port already in use.
**Verification:** Exit code is non-zero.
**Cleanup:** Kill the remote nc.

#### TC-TRL-05: Open reverse tunnel with invalid spec

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel listen invalid_spec`
**Expected Output:** Error about invalid tunnel spec.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-TRL-06: Reverse tunnel with --connection flag

**Category:** Happy Path
**Prerequisites:** Multiple connections active.
**Steps:**
1. Run `distant tunnel listen --connection <ID> 9096:3006`
**Expected Output:** Reverse tunnel on specified connection.
**Verification:** `distant tunnel list --connection <ID>` shows tunnel.
**Cleanup:** `distant tunnel close <ID>`

#### TC-TRL-07: Reverse tunnel bidirectional data flow

**Category:** Happy Path
**Prerequisites:** Active connection. Local echo service.
**Steps:**
1. Start local echo listener:
   `(nc -l 3007 | while read line; do echo "local-echo: $line"; done) &`
2. Run `distant tunnel listen 9097:3007`
3. From remote: `distant spawn -- sh -c 'echo "test" | nc localhost 9097'`
**Expected Output:** Bidirectional data through reverse tunnel.
**Verification:** Response from local echo received on remote.
**Cleanup:** `distant tunnel close <ID>`. Kill nc.

#### TC-TRL-08: Open multiple reverse tunnels

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel listen 9098:3008`
2. Run `distant tunnel listen 9099:3009`
3. Run `distant tunnel list`
**Expected Output:** Both tunnels listed.
**Verification:** Two entries in tunnel list.
**Cleanup:** Close both tunnels.

#### TC-TRL-09: Open reverse tunnel when no connection active

**Category:** Error Handling
**Prerequisites:** Manager running but no active connections.
**Steps:**
1. Run `distant tunnel listen 9100:3010`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant tunnel close`

### Purpose

Close an active tunnel by its ID.

### Dependencies

- `nc` — verify traffic stops after close

### Test Harness Equivalents

- **`test-tcp-peer`** — verify connection failure after tunnel close

### Test Cases

#### TC-TRC-01: Close an active forward tunnel

**Category:** Happy Path
**Prerequisites:** Active connection with an open forward tunnel (TC-TFO-01).
Note the tunnel ID.
**Steps:**
1. Open tunnel: `distant tunnel open 8100:9100`
2. Note the tunnel ID from output.
3. Run `distant tunnel close <TUNNEL_ID>`
**Expected Output:** Tunnel closed confirmation.
**Verification:** `distant tunnel list` no longer shows the tunnel.
`nc localhost 8100` fails (connection refused).
**Cleanup:** None

#### TC-TRC-02: Close an active reverse tunnel

**Category:** Happy Path
**Prerequisites:** Active connection with an open reverse tunnel.
**Steps:**
1. Open tunnel: `distant tunnel listen 9101:3011`
2. Note the tunnel ID.
3. Run `distant tunnel close <TUNNEL_ID>`
**Expected Output:** Tunnel closed.
**Verification:** `tunnel list` no longer shows it. Remote port 9101 no longer
accepts connections.
**Cleanup:** None

#### TC-TRC-03: Close with invalid/nonexistent ID

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant tunnel close 999999`
**Expected Output:** Error about tunnel not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-TRC-04: Close already-closed tunnel

**Category:** Error Handling
**Prerequisites:** A tunnel that was previously closed.
**Steps:**
1. Open and close a tunnel:
   `distant tunnel open 8102:9102`
   `distant tunnel close <ID>`
2. Run `distant tunnel close <same ID>` again
**Expected Output:** Error about tunnel not found or already closed.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-TRC-05: Verify traffic stops after close

**Category:** Happy Path
**Prerequisites:** Active forward tunnel with verified traffic flow.
**Steps:**
1. Open tunnel and verify traffic flows (per TC-TFO-01)
2. Close the tunnel: `distant tunnel close <ID>`
3. Attempt to connect: `nc localhost <LOCAL_PORT>`
**Expected Output:** Connection refused after tunnel is closed.
**Verification:** `nc` fails to connect.
**Cleanup:** None

---

## Command: `distant tunnel list`

### Purpose

List all active tunnels.

### Dependencies

None beyond active connection.

### Test Harness Equivalents

None needed — output is self-verifying.

### Test Cases

#### TC-TLS-01: List with no tunnels

**Category:** Happy Path
**Prerequisites:** Active connection. No tunnels open.
**Steps:**
1. Run `distant tunnel list`
**Expected Output:** Empty list or message indicating no active tunnels.
**Verification:** No tunnel entries in output.
**Cleanup:** None

#### TC-TLS-02: List with active tunnels

**Category:** Happy Path
**Prerequisites:** Active connection with open tunnels (both forward and
reverse).
**Steps:**
1. Open tunnels:
   `distant tunnel open 8110:9110`
   `distant tunnel listen 9111:3011`
2. Run `distant tunnel list`
**Expected Output:** Lists both tunnels with IDs, types (forward/reverse),
local/remote port mappings.
**Verification:** Two entries shown. IDs, ports, and directions match what
was opened.
**Cleanup:** Close both tunnels.

#### TC-TLS-03: List with --connection filter

**Category:** Happy Path
**Prerequisites:** Multiple connections with tunnels on different connections.
**Steps:**
1. Open tunnel on connection A: `distant tunnel open --connection <ID_A> 8112:9112`
2. Open tunnel on connection B: `distant tunnel open --connection <ID_B> 8113:9113`
3. Run `distant tunnel list --connection <ID_A>`
**Expected Output:** Only tunnel on connection A listed.
**Verification:** One entry matching connection A.
**Cleanup:** Close both tunnels.

#### TC-TLS-04: List when no connection active

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `distant tunnel list`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

### 5E. Process Execution

---

## Command: `distant shell`

### Purpose

Open an interactive remote shell process, with optional predictive local echo
for reduced perceived latency.

### Dependencies

- Remote shell (`/bin/sh`, `/bin/bash`, `cmd.exe`)
- Terminal emulator (tester's terminal)

### Test Harness Equivalents

None needed for manual testing — requires interactive terminal.

### Test Cases

#### TC-SHL-01: Open default shell

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell`
**Expected Output:**

```
┌─────────────────────────────────┐
│ $ distant shell                  │
│ user@host:~$                    │
│ $ echo "in remote shell"        │
│ in remote shell                  │
│ $ exit                           │
└─────────────────────────────────┘
```

**Verification:** Interactive prompt appears. Commands execute. `exit` returns
to local shell.
**Cleanup:** None (exited cleanly).

#### TC-SHL-02: Open shell with custom command

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell -- /bin/sh`
**Expected Output:**

```
┌─────────────────────────────────┐
│ $ distant shell -- /bin/sh       │
│ $                                │
│ $ echo "hello from sh"          │
│ hello from sh                    │
│ $ exit                           │
└─────────────────────────────────┘
```

**Verification:** `/bin/sh` shell opens (not the default shell).
**Cleanup:** None

#### TC-SHL-03: Shell with --current-dir

**Category:** Happy Path
**Prerequisites:** Active connection. `/tmp` exists.
**Steps:**
1. Run `distant shell --current-dir /tmp`
2. Type `pwd` in the remote shell
**Expected Output:** `pwd` returns `/tmp`.
**Verification:** Working directory is `/tmp`.
**Cleanup:** `exit`

#### TC-SHL-04: Shell with --environment

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --environment 'MY_TEST_VAR=42'`
2. Type `echo $MY_TEST_VAR` in the remote shell
**Expected Output:** `42`
**Verification:** Environment variable is set.
**Cleanup:** `exit`

#### TC-SHL-05: Shell with --predict off

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --predict off`
2. Type commands in the shell
**Expected Output:** All output comes strictly from the server. No local echo.
**Verification:** Shell works normally. Latency may be perceptible on high-
latency connections.
**Cleanup:** `exit`

#### TC-SHL-06: Shell with --predict on

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --predict on`
2. Type characters
**Expected Output:** Characters appear immediately (local prediction). Server
confirms or corrects.
**Verification:** Typing feels instant. No visual artifacts on correction.
**Cleanup:** `exit`

#### TC-SHL-07: Shell with --predict adaptive

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --predict adaptive`
2. Type commands
**Expected Output:** Prediction activates when SRTT > 30ms. On localhost,
likely stays off.
**Verification:** Shell works normally.
**Cleanup:** `exit`

#### TC-SHL-08: Shell with --predict fast

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --predict fast`
2. Type commands
**Expected Output:** Aggressive prediction. Characters echoed immediately
without waiting for epoch confirmation.
**Verification:** Typing is very responsive.
**Cleanup:** `exit`

#### TC-SHL-09: Shell with --predict fast-adaptive

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell --predict fast-adaptive`
2. Type commands
**Expected Output:** Adaptive + fast epoch. Prediction activates on high
latency, skips epoch confirmation when active.
**Verification:** Shell works normally.
**Cleanup:** `exit`

#### TC-SHL-10: Shell with --connection flag

**Category:** Happy Path
**Prerequisites:** Multiple active connections.
**Steps:**
1. Run `distant shell --connection <ID>`
2. Verify which host you're on (e.g., `hostname`)
**Expected Output:** Shell opens on the specified connection's remote.
**Verification:** Hostname matches expected remote.
**Cleanup:** `exit`

#### TC-SHL-11: Shell exit code propagation

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant shell -- /bin/sh`
2. Type `exit 42`
3. Check `echo $?`
**Expected Output:** Shell exits with code 42. `$?` reflects the remote exit
code.
**Verification:** Exit code is 42.
**Cleanup:** None

#### TC-SHL-12: Shell with no active connection

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `distant shell`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant spawn`

### Purpose

Spawn a process on the remote machine. Supports PTY mode, shell wrapping, LSP
proxy mode, environment variables, and predictive echo.

### Dependencies

- Remote commands (`echo`, `cat`, `sleep`, `ls`, etc.)
- LSP server binary (for `--lsp` tests)

### Test Harness Equivalents

- **`test-sleep`** — long-running process for lifecycle tests
- **`test-tcp-peer`** — verify LSP proxy connections

### Test Cases

#### TC-SPN-01: Spawn simple command

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- echo "hello world"`
**Expected Output:** `hello world`
**Verification:** Exact output match. Exit code 0.
**Cleanup:** None

#### TC-SPN-02: Spawn command with arguments

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- ls /tmp`
**Expected Output:** Listing of `/tmp` directory.
**Verification:** Output is a directory listing.
**Cleanup:** None

#### TC-SPN-03: Spawn command with exit code

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- sh -c "exit 42"`
2. Check `echo $?`
**Expected Output:** No stdout. Exit code is 42.
**Verification:** `$?` is 42.
**Cleanup:** None

#### TC-SPN-04: Spawn with --pty

**Category:** Happy Path
**Prerequisites:** Active connection. PTY support available.
**Steps:**
1. Run `distant spawn --pty -- ls /tmp`
**Expected Output:** Output includes terminal-formatted listing (may include
colors/control codes).
**Verification:** Output appears. Process ran with a PTY.
**Cleanup:** None

#### TC-SPN-05: Spawn with --shell (default shell)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --shell -- -c "echo from_shell"`
**Expected Output:** `from_shell`
**Verification:** Command was executed within the user's default shell.
**Cleanup:** None

#### TC-SPN-06: Spawn with --shell specifying shell

**Category:** Happy Path
**Prerequisites:** Active connection. `/bin/sh` available.
**Steps:**
1. Run `distant spawn --shell /bin/sh -- -c "echo from_sh"`
**Expected Output:** `from_sh`
**Verification:** Command ran in `/bin/sh`.
**Cleanup:** None

#### TC-SPN-07: Spawn with -c/--cmd flag

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --cmd "echo hello from cmd"`
**Expected Output:** `hello from cmd`
**Verification:** Command string was interpreted correctly.
**Cleanup:** None

#### TC-SPN-08: Spawn with --environment

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --environment 'TEST_VAR=value123' -- printenv TEST_VAR`
**Expected Output:** `value123`
**Verification:** Environment variable set on remote process.
**Cleanup:** None

#### TC-SPN-09: Spawn with --current-dir

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --current-dir /tmp -- pwd`
**Expected Output:** `/tmp`
**Verification:** Working directory was `/tmp`.
**Cleanup:** None

#### TC-SPN-10: Spawn with --lsp (LSP proxy mode)

**Category:** Happy Path
**Prerequisites:** Active connection. An LSP server binary available on the
remote (e.g., a simple test LSP server or `rust-analyzer`).
**Steps:**
1. Run `distant spawn --lsp -- <lsp-server-binary>`
2. Send an LSP initialize request via stdin
**Expected Output:** LSP server starts. Path translation occurs between local
`distant://` paths and remote paths.
**Verification:** LSP initialize response received.
**Cleanup:** Send shutdown request or Ctrl+C.

#### TC-SPN-11: Spawn with --lsp and custom scheme

**Category:** Happy Path
**Prerequisites:** Same as TC-SPN-10.
**Steps:**
1. Run `distant spawn --lsp myscheme -- <lsp-server-binary>`
**Expected Output:** LSP proxy translates paths to `myscheme://` scheme instead
of default `distant://`.
**Verification:** Path translation uses custom scheme.
**Cleanup:** Shutdown or Ctrl+C.

#### TC-SPN-12: Spawn with --predict off

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --predict off --pty -- /bin/sh`
2. Type commands
**Expected Output:** No local prediction. All output from server.
**Verification:** Shell works normally.
**Cleanup:** `exit`

#### TC-SPN-13: Spawn with --predict on (PTY mode)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn --predict on --pty -- /bin/sh`
2. Type characters
**Expected Output:** Local echo of keystrokes before server confirmation.
**Verification:** Characters appear immediately.
**Cleanup:** `exit`

#### TC-SPN-14: Spawn stdin pipe

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `echo "input data" | distant spawn -- cat`
**Expected Output:** `input data`
**Verification:** Stdin was forwarded to remote process.
**Cleanup:** None

#### TC-SPN-15: Spawn stderr output

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- sh -c "echo error >&2"`
**Expected Output:** `error` on stderr.
**Verification:** Stderr output appears (may be interleaved with terminal).
**Cleanup:** None

#### TC-SPN-16: Spawn long-running process

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- sleep 5` (wait for it to complete)
**Expected Output:** Process runs for 5 seconds then exits.
**Verification:** Exit code 0. Process took approximately 5 seconds.
**Cleanup:** None

#### TC-SPN-17: Spawn with --connection flag

**Category:** Happy Path
**Prerequisites:** Multiple active connections.
**Steps:**
1. Run `distant spawn --connection <ID> -- hostname`
**Expected Output:** Hostname of the remote connected via the specified ID.
**Verification:** Hostname matches expected.
**Cleanup:** None

#### TC-SPN-18: Spawn command that does not exist

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn -- /nonexistent/binary`
**Expected Output:** Error about command not found.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-SPN-19: Spawn with no active connection

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `distant spawn -- echo hi`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-SPN-20: Spawn with no command specified

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant spawn`
**Expected Output:** Error about no command provided, or opens default behavior.
Document actual behavior.
**Verification:** Note behavior.
**Cleanup:** Ctrl+C if needed.

---

### 5F. Information & API

---

## Command: `distant system-info`

### Purpose

Retrieve system information about the remote machine (networking configuration,
OS details).

### Dependencies

None beyond active connection.

### Test Harness Equivalents

None needed — output is self-verifying.

### Test Cases

#### TC-SYS-01: Get system info

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant system-info`
**Expected Output:** System information including OS family, architecture,
hostname, and other networking/system details.
**Verification:** Output contains recognizable system information.
**Cleanup:** None

#### TC-SYS-02: Get system info with --connection

**Category:** Happy Path
**Prerequisites:** Multiple active connections.
**Steps:**
1. Run `distant system-info --connection <ID>`
**Expected Output:** System info from the specified connection's remote.
**Verification:** Info matches expected remote.
**Cleanup:** None

#### TC-SYS-03: Get system info with no connection

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `distant system-info`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant version`

### Purpose

Retrieve version information of the remote server.

### Dependencies

- `jq` — parse JSON format output

### Test Harness Equivalents

- **`test-json-check`** — validate JSON output

### Test Cases

#### TC-VER-01: Get version (shell format)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant version`
**Expected Output:** Human-readable version string of the remote server.
**Verification:** Output contains a version number.
**Cleanup:** None

#### TC-VER-02: Get version (JSON format)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant version --format json`
**Expected Output:** JSON object containing version information.
**Verification:** Valid JSON. Contains version field.
**Cleanup:** None

#### TC-VER-03: Get version with --connection

**Category:** Happy Path
**Prerequisites:** Multiple active connections.
**Steps:**
1. Run `distant version --connection <ID>`
**Expected Output:** Version from the specified connection's server.
**Verification:** Version returned.
**Cleanup:** None

#### TC-VER-04: Get version with no connection

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `distant version`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

---

## Command: `distant api`

### Purpose

Listen over stdin and stdout to communicate with a distant server using the
JSON lines API. Each line of input is a JSON request; each line of output is a
JSON response.

### Dependencies

- `jq` — construct and parse JSON messages
- `echo` / `printf` — send JSON to stdin

### Test Harness Equivalents

- **`test-json-check`** — validate JSON responses

### Test Cases

#### TC-API-01: Send a basic request

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Start the API:
   `echo '{"type":"version"}' | distant api`
**Expected Output:** JSON response line with version information.
**Verification:** Valid JSON response. Contains version data.
**Cleanup:** None (process exits on EOF).

#### TC-API-02: Multiple requests in sequence

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Send multiple requests:
   ```
   printf '{"type":"version"}\n{"type":"system_info"}\n' | distant api
   ```
**Expected Output:** Two JSON response lines, one for each request.
**Verification:** Each response is valid JSON. First contains version, second
contains system info.
**Cleanup:** None

#### TC-API-03: File write then read via API

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Send write request then read request:
   ```
   printf '{"type":"file_write","path":"/tmp/api_test.txt","data":"api data"}\n{"type":"file_read","path":"/tmp/api_test.txt"}\n' | distant api
   ```
**Expected Output:** Write response (success), then read response with content
"api data".
**Verification:** Read response contains the written data.
**Cleanup:** `distant fs remove /tmp/api_test.txt`

#### TC-API-04: Malformed JSON input

**Category:** Error Handling
**Prerequisites:** Active connection.
**Steps:**
1. Run `echo 'not json at all' | distant api`
**Expected Output:** JSON error response about malformed input.
**Verification:** Error response is valid JSON with error details.
**Cleanup:** None

#### TC-API-05: API with --timeout

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `echo '{"type":"version"}' | distant api --timeout 5`
**Expected Output:** Response received within 5 seconds.
**Verification:** Same behavior as without timeout, but with a 5-second limit.
**Cleanup:** None

#### TC-API-06: API with --connection flag

**Category:** Happy Path
**Prerequisites:** Multiple active connections.
**Steps:**
1. Run `echo '{"type":"version"}' | distant api --connection <ID>`
**Expected Output:** Response from the specified connection's server.
**Verification:** Valid JSON response.
**Cleanup:** None

#### TC-API-07: API with no connection

**Category:** Error Handling
**Prerequisites:** Manager running but no connections.
**Steps:**
1. Run `echo '{"type":"version"}' | distant api`
**Expected Output:** Error about no active connection.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-API-08: API interactive session (long-running)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Start `distant api` interactively (not piped)
2. Type a JSON request line and press Enter
3. Observe response
4. Type another request
5. Ctrl+C or close stdin
**Expected Output:** Responses appear after each request line. Process runs
until stdin is closed.
**Verification:** Multiple request-response cycles work.
**Cleanup:** Ctrl+C.

#### TC-API-09: API with empty input

**Category:** Edge Case
**Prerequisites:** Active connection.
**Steps:**
1. Run `echo "" | distant api`
**Expected Output:** No response (empty line ignored) or error for empty input.
**Verification:** Process handles gracefully.
**Cleanup:** None

---

## 6. Global Options Tests

These tests verify options that are common across most or all commands.

### Dependencies

- `cat` — read log files
- `jq` — parse JSON in log output

### Test Harness Equivalents

- **`test-file-read`** — read log files

### Test Cases

#### TC-GLO-01: --log-level off

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --log-level off version`
**Expected Output:** Version output with no log messages.
**Verification:** No log output on stderr.
**Cleanup:** None

#### TC-GLO-02: --log-level error

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --log-level error version`
**Expected Output:** Version output. Only error-level logs (if any) on stderr.
**Verification:** No debug/info/warn messages.
**Cleanup:** None

#### TC-GLO-03: --log-level debug

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --log-level debug version`
**Expected Output:** Version output plus verbose debug log messages on stderr.
**Verification:** Debug-level messages appear.
**Cleanup:** None

#### TC-GLO-04: --log-level trace

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --log-level trace version`
**Expected Output:** Very verbose trace-level logging on stderr.
**Verification:** Trace messages appear (more than debug).
**Cleanup:** None

#### TC-GLO-05: --log-file

**Category:** Happy Path
**Prerequisites:** Active connection. Writable temp directory.
**Steps:**
1. Run `distant --log-level debug --log-file /tmp/distant_test.log version`
2. Read log file: `cat /tmp/distant_test.log`
**Expected Output:** Version output on stdout. Log messages written to file
instead of stderr.
**Verification:** Log file contains debug messages. Stderr is clean.
**Cleanup:** `rm /tmp/distant_test.log`

#### TC-GLO-06: --quiet

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --quiet launch ssh://localhost`
**Expected Output:** Connection ID printed but no spinners or informational
status messages on stderr.
**Verification:** Stderr is empty or minimal. Connection still established.
**Cleanup:** `distant kill`

#### TC-GLO-07: --config (custom config file)

**Category:** Happy Path
**Prerequisites:** Generate a config file first.
**Steps:**
1. Generate: `distant generate config --output /tmp/test_distant.toml`
2. Run `distant --config /tmp/test_distant.toml version`
**Expected Output:** Command runs using the specified config file.
**Verification:** No error about config. Command succeeds.
**Cleanup:** `rm /tmp/test_distant.toml`

#### TC-GLO-08: --config with nonexistent file

**Category:** Error Handling
**Prerequisites:** None.
**Steps:**
1. Run `distant --config /tmp/nonexistent_config.toml version`
**Expected Output:** Error about config file not found, or graceful fallback
to defaults.
**Verification:** Document actual behavior (error vs. fallback).
**Cleanup:** None

#### TC-GLO-09: --cache (custom cache path)

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant launch --cache /tmp/test_cache.toml ssh://localhost`
**Expected Output:** Connection established. Cache stored at custom path.
**Verification:** `/tmp/test_cache.toml` exists after command.
**Cleanup:** `distant kill && rm /tmp/test_cache.toml`

#### TC-GLO-10: --unix-socket (custom manager socket, Unix)

**Category:** Happy Path
**Prerequisites:** Unix platform. Manager started with custom socket
(TC-MGR-08).
**Steps:**
1. Start manager: `distant manager listen --unix-socket /tmp/custom.sock --daemon`
2. Run `distant --unix-socket /tmp/custom.sock manager version`
**Expected Output:** Manager version returned using custom socket.
**Verification:** Command succeeds via custom socket path.
**Cleanup:** Kill manager. `rm /tmp/custom.sock`

#### TC-GLO-11: --windows-pipe (custom named pipe, Windows)

**Category:** Cross-Platform
**Prerequisites:** Windows platform. Manager started with custom pipe name.
**Steps:**
1. Start manager: `distant manager listen --windows-pipe test_pipe`
2. Run `distant --windows-pipe test_pipe manager version`
**Expected Output:** Manager version returned using custom named pipe.
**Verification:** Command succeeds.
**Cleanup:** Stop manager.

#### TC-GLO-12: --log-level with invalid value

**Category:** Error Handling
**Prerequisites:** None.
**Steps:**
1. Run `distant --log-level invalid version`
**Expected Output:** Error about invalid log level. Lists valid values.
**Verification:** Exit code is non-zero.
**Cleanup:** None

#### TC-GLO-13: --quiet with --log-level

**Category:** Happy Path
**Prerequisites:** Active connection.
**Steps:**
1. Run `distant --quiet --log-level debug version`
**Expected Output:** Version output. Debug logs may appear (--quiet suppresses
informational messages, not log output).
**Verification:** Informational spinners suppressed. Debug log messages may
still appear on stderr.
**Cleanup:** None

---

## 7. Integration Scenarios

These multi-step scenarios test realistic workflows combining multiple commands.

### Dependencies

All dependencies from individual command sections apply.

### Test Harness Equivalents

All harness equivalents from individual sections apply.

### Test Cases

#### TC-INT-01: Full lifecycle (manager → launch → use → kill → stop)

**Category:** Integration
**Prerequisites:** Local `sshd` running. No manager running.
**Steps:**
1. Start manager: `distant manager listen --daemon`
2. Launch server: `distant launch ssh://localhost`
3. Check status: `distant status`
4. Run a command: `distant spawn -- echo "lifecycle test"`
5. Write a file: `distant fs write /tmp/int_test.txt "integration"`
6. Read the file: `distant fs read /tmp/int_test.txt`
7. Open a shell briefly: `distant shell -- /bin/sh -c "echo ok && exit"`
8. Kill the connection: `distant kill`
9. Verify: `distant status` shows no connections
10. Stop manager (kill the daemon PID)
**Expected Output:** Each step succeeds. Output of step 4 is "lifecycle test".
Step 6 returns "integration". Step 7 returns "ok". Status after kill shows no
connections.
**Verification:** All steps complete without error.
**Cleanup:** Kill manager if still running. `distant fs remove /tmp/int_test.txt`
(if not already cleaned by kill).

#### TC-INT-02: Multi-connection management

**Category:** Integration
**Prerequisites:** Manager running. Local `sshd`.
**Steps:**
1. Connect first: `distant launch ssh://localhost` → note ID_A
2. Connect second: `distant launch --distant-bind-server any ssh://localhost` → note ID_B
3. Status: `distant status` shows two connections
4. Select first: `distant select <ID_A>`
5. Run command: `distant spawn -- echo "on connection A"`
6. Select second: `distant select <ID_B>`
7. Run command: `distant spawn -- echo "on connection B"`
8. Kill first: `distant kill <ID_A>`
9. Status: only ID_B remains
10. Kill second: `distant kill <ID_B>`
**Expected Output:** Both connections work independently. Selecting between
them changes which remote receives commands.
**Verification:** Each step produces expected output.
**Cleanup:** None (all connections killed).

#### TC-INT-03: File roundtrip (write → read → copy local → verify → remove)

**Category:** Integration
**Prerequisites:** Active connection.
**Steps:**
1. Write: `distant fs write /tmp/roundtrip.txt "roundtrip data"`
2. Read: `distant fs read /tmp/roundtrip.txt` → verify "roundtrip data"
3. Copy to remote: `distant fs copy /tmp/roundtrip.txt /tmp/roundtrip_copy.txt`
4. Read copy: `distant fs read /tmp/roundtrip_copy.txt` → verify same content
5. Download: `distant copy :/tmp/roundtrip_copy.txt /tmp/local_roundtrip.txt`
6. Read local: `cat /tmp/local_roundtrip.txt` → verify "roundtrip data"
7. Cleanup:
   `distant fs remove /tmp/roundtrip.txt`
   `distant fs remove /tmp/roundtrip_copy.txt`
   `rm /tmp/local_roundtrip.txt`
**Expected Output:** Data integrity maintained through all operations.
**Verification:** Content is "roundtrip data" at every step.
**Cleanup:** Steps 7 above.

#### TC-INT-04: Error recovery (kill server mid-operation)

**Category:** Integration
**Prerequisites:** Active connection. Know the server's PID or process.
**Steps:**
1. Start a long-running watch: `distant fs watch /tmp` (in terminal A)
2. Kill the server process (from local shell, kill the distant server PID)
3. Observe terminal A behavior
4. Try another command: `distant spawn -- echo "after kill"`
**Expected Output:** The watch in terminal A should error or disconnect
gracefully. The spawn in step 4 should fail with a connection error.
**Verification:** Client does not hang indefinitely. Error messages are
informative.
**Cleanup:** Re-establish connection if needed for further tests.

#### TC-INT-05: Tunnel lifecycle workflow

**Category:** Integration
**Prerequisites:** Active connection.
**Steps:**
1. Start a local listener: `echo "tunnel-data" | nc -l 3020 &`
2. Open forward tunnel: `distant tunnel open 8020:3020` → note TUNNEL_ID_A
3. Open reverse tunnel: `distant tunnel listen 9020:3021` → note TUNNEL_ID_B
4. List tunnels: `distant tunnel list` → shows both
5. Test forward tunnel: `nc localhost 8020` → receives "tunnel-data"
6. Close forward tunnel: `distant tunnel close <TUNNEL_ID_A>`
7. Verify forward stopped: `nc localhost 8020` fails
8. List tunnels: only reverse tunnel remains
9. Close reverse tunnel: `distant tunnel close <TUNNEL_ID_B>`
10. List tunnels: empty
**Expected Output:** Full tunnel lifecycle works. Traffic flows when open,
stops when closed.
**Verification:** Each step produces expected result.
**Cleanup:** Kill any remaining nc processes.

#### TC-INT-06: SSH shortcut workflow

**Category:** Integration
**Prerequisites:** Local `sshd`. No manager needed (auto-starts).
**Steps:**
1. Run: `distant ssh localhost -- echo "step 1"`
2. Run: `distant ssh localhost -- cat /etc/hostname`
3. Open interactive: `distant ssh localhost`, type `ls /tmp`, then `exit`
4. Run with env: `distant ssh --environment 'X=1' localhost -- printenv X`
**Expected Output:** Step 1: "step 1". Step 2: hostname. Step 3: directory
listing. Step 4: "1".
**Verification:** Each command succeeds.
**Cleanup:** None

#### TC-INT-07: Search and process files

**Category:** Integration
**Prerequisites:** Active connection.
**Steps:**
1. Setup test directory:
   `distant fs make-dir --all /tmp/search_int/src`
   `distant fs write /tmp/search_int/src/main.rs "fn main() { println!(\"hello\"); }"`
   `distant fs write /tmp/search_int/src/lib.rs "pub fn greet() { println!(\"hi\"); }"`
   `distant fs write /tmp/search_int/README.md "# Project"`
2. Search for "println": `distant fs search "println" /tmp/search_int`
3. Search by path: `distant fs search --target path "rs$" /tmp/search_int`
4. Get metadata: `distant fs metadata /tmp/search_int/src/main.rs`
5. Read directory recursively: `distant fs read --depth 0 --absolute /tmp/search_int`
**Expected Output:** Step 2: matches in both .rs files. Step 3: main.rs and
lib.rs. Step 4: file metadata. Step 5: full directory tree.
**Verification:** All operations succeed with expected results.
**Cleanup:** `distant fs remove --force /tmp/search_int`

#### TC-INT-08: Generate config and use it

**Category:** Integration
**Prerequisites:** None (no server needed for generation).
**Steps:**
1. Generate config: `distant generate config --output /tmp/int_config.toml`
2. Start manager with config: `distant --config /tmp/int_config.toml manager listen --daemon`
3. Verify manager: `distant --config /tmp/int_config.toml manager version`
4. Stop manager (kill daemon)
**Expected Output:** Config generated, used by manager, manager responds.
**Verification:** Each step succeeds.
**Cleanup:** Kill daemon. `rm /tmp/int_config.toml`

#### TC-INT-09: Service install → start → use → stop → uninstall

**Category:** Integration
**Prerequisites:** Elevated privileges. No existing service.
**Steps:**
1. Install: `distant manager service install --user`
2. Start: `distant manager service start --user`
3. Verify: `distant manager version` succeeds
4. Launch: `distant launch ssh://localhost`
5. Use: `distant spawn -- echo "service test"`
6. Kill connection: `distant kill`
7. Stop: `distant manager service stop --user`
8. Uninstall: `distant manager service uninstall --user`
**Expected Output:** Full service lifecycle works end-to-end.
**Verification:** Each step succeeds. After uninstall, service is gone.
**Cleanup:** Steps 7-8 above.

#### TC-INT-10: Concurrent file operations

**Category:** Integration
**Prerequisites:** Active connection.
**Steps:**
1. Write multiple files rapidly:
   `distant fs write /tmp/conc_1.txt "one"`
   `distant fs write /tmp/conc_2.txt "two"`
   `distant fs write /tmp/conc_3.txt "three"`
2. Read all back:
   `distant fs read /tmp/conc_1.txt`
   `distant fs read /tmp/conc_2.txt`
   `distant fs read /tmp/conc_3.txt`
3. Rename chain:
   `distant fs rename /tmp/conc_1.txt /tmp/conc_1_renamed.txt`
4. Remove all:
   `distant fs remove /tmp/conc_1_renamed.txt`
   `distant fs remove /tmp/conc_2.txt`
   `distant fs remove /tmp/conc_3.txt`
**Expected Output:** All operations succeed. Content matches. Renames work.
Removes clean up.
**Verification:** Each read returns expected content.
**Cleanup:** None (step 4 is cleanup).

---

## Appendix A: Test Case Summary

| Section | Prefix | Count |
|---------|--------|------:|
| generate config | GEN | 3 |
| generate completion | GEN | 7 |
| server listen | SRV | 18 |
| manager listen | MGR | 9 |
| manager version | MGR | 3 |
| manager service | SVC | 10 |
| launch | LCH | 13 |
| connect | CON | 8 |
| ssh | SSH | 15 |
| status | STA | 7 |
| kill | KIL | 5 |
| select | SEL | 5 |
| copy (local↔remote) | CPY | 10 |
| fs copy | FSC | 5 |
| fs exists | FSE | 5 |
| fs make-dir | FSD | 5 |
| fs metadata | FSM | 7 |
| fs read | FSR | 10 |
| fs remove | FSRM | 7 |
| fs rename | FSRN | 5 |
| fs search | FSS | 14 |
| fs set-permissions | FSP | 10 |
| fs watch | FSW | 9 |
| fs write | FSWR | 8 |
| tunnel open | TFO | 10 |
| tunnel listen | TRL | 9 |
| tunnel close | TRC | 5 |
| tunnel list | TLS | 4 |
| shell | SHL | 12 |
| spawn | SPN | 20 |
| system-info | SYS | 3 |
| version | VER | 4 |
| api | API | 9 |
| Global Options | GLO | 13 |
| Integration | INT | 10 |
| **Total** | | **297** |

## Appendix B: Cross-Platform Test Matrix

Mark each test case with its applicability:

| Symbol | Meaning |
|--------|---------|
| A | All platforms |
| U | Unix only (Linux, macOS, FreeBSD, NetBSD, OpenBSD) |
| W | Windows only |
| L | Linux only |
| M | macOS only |
| B | BSD only (FreeBSD, NetBSD, OpenBSD) |
| D | Requires Docker |

**Tests requiring platform-specific attention:**

| Test | Platforms | Notes |
|------|-----------|-------|
| TC-SRV-10 (daemon) | U | `--daemon` forks on Unix, N/A on Windows |
| TC-MGR-03..05 (access) | U | Unix socket permissions |
| TC-SVC-01..10 (service) | Varies | Per platform service manager |
| TC-FSP-01,02,05,09,10 (permissions) | U | POSIX mode bits |
| TC-FSP-03,04 (readonly) | A | Works on all platforms |
| TC-FSM-03..06 (symlink) | U | Symlinks require privileges on Windows |
| TC-FSE-04,05 (symlink) | U | Symlink existence checks |
| TC-GLO-10 (unix-socket) | U | Unix-only option |
| TC-GLO-11 (windows-pipe) | W | Windows-only option |
