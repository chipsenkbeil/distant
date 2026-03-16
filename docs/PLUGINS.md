# Distant Plugin Binary Specification

This document describes the protocol that external plugin binaries must implement to integrate with the distant manager. The manager communicates with plugin binaries via stdin/stdout using a JSON-lines protocol.

## Overview

A plugin binary is a standalone executable that handles `launch` and/or `connect` operations for one or more URI schemes. The distant manager spawns the binary as a child process, passing arguments via the command line and exchanging messages via JSON-lines on stdin/stdout.

## Registration

Plugins are registered via `~/.config/distant/plugins.toml`:

```toml
[plugins.docker]
path = "/usr/local/bin/distant-plugin-docker"

[plugins.ftp]
path = "/usr/local/bin/distant-plugin-ftp"
schemes = ["ftp", "sftp"]
```

Or via the `--plugin` CLI flag:

```
distant manager listen --plugin docker=/usr/local/bin/distant-plugin-docker
```

**Fields:**
- `path` (required): Absolute path to the plugin binary.
- `schemes` (optional): List of URI schemes this plugin handles. Defaults to `["<name>"]` where `<name>` is the TOML key or CLI flag name.

## Subcommands

The binary must accept two subcommands: `launch` and `connect`. If a subcommand is not supported, the binary should exit with a non-zero code and write an error message.

### Common Arguments

Both subcommands receive:
- **Positional argument**: The destination URI (e.g., `docker://container-name`).
- **Key-value flags**: Map entries from the distant options, passed as `--key=value` CLI flags.

### `launch`

```
binary launch <destination> [--key=value ...]
```

Short-lived process. Performs the launch operation (e.g., start a server on a remote host) and exits.

**Lifecycle:**
1. Binary starts and performs setup.
2. Auth relay phase (see below) — exchange authentication challenges/responses.
3. On success: write `{"destination": "scheme://host:port"}` to stdout and exit with code 0.
4. On failure: write `{"error": {"kind": "...", "description": "..."}}` to stdout and exit with non-zero code.

### `connect`

```
binary connect <destination> [--key=value ...]
```

Long-lived process. Acts as a bidirectional distant API proxy.

**Lifecycle:**
1. Binary starts and performs setup.
2. Auth relay phase (see below).
3. On ready: write `{"ready": true}` to stdout.
4. After ready, the process becomes a bidirectional JSON-lines transport:
   - Manager writes distant API requests as JSON-lines to the binary's stdin.
   - Binary writes distant API responses as JSON-lines to stdout.
   - The process stays alive for the lifetime of the connection.
5. On setup failure: write `{"error": {...}}` and exit with non-zero code.

## Authentication Relay

During the setup phase of both `launch` and `connect`, the binary can request interactive authentication from the user via the manager.

**Binary writes** (challenge request):
```json
{"auth_challenge": {"questions": [{"text": "Password:", "label": "ssh-prompt", "options": {"echo": "false"}}], "options": {"instructions": "", "username": "user"}}}
```

**Manager responds** (on stdin):
```json
{"auth_response": {"answers": ["hunter2"]}}
```

Each `auth_challenge` must receive exactly one `auth_response` before the binary continues. Multiple rounds of challenge/response are supported.

### Auth Challenge Fields

- `questions` (array): List of questions to present to the user.
  - `text` (string): The prompt text.
  - `label` (string, optional): Machine-readable label (e.g., `"ssh-prompt"`).
  - `options` (object, optional): Key-value metadata (e.g., `{"echo": "false"}`).
- `options` (object, optional): Top-level metadata (e.g., `{"instructions": "...", "username": "..."}`).

## Error Format

```json
{"error": {"kind": "not_found", "description": "Container not found"}}
```

**`kind`** values (mapped to `io::ErrorKind`):
- `"not_found"` — NotFound
- `"permission_denied"` — PermissionDenied
- `"connection_refused"` — ConnectionRefused
- `"unsupported"` — Unsupported
- `"other"` (default) — Other

**`description`**: Human-readable error message.

## Timeouts

The manager enforces a 120-second timeout on the setup phase (everything before `{"ready": true}` or `{"destination": ...}`). If the binary does not complete setup within this window, the process is killed and the operation fails with a timeout error.

## Platform Notes

The protocol is platform-independent: stdin/stdout JSON-lines works identically on Linux, macOS, and Windows. No platform-specific IPC mechanisms are required.

## Built-in Plugins

Distant ships with three built-in plugins compiled directly into the binary
(enabled via Cargo features):

| Plugin | Feature | Schemes | Description |
|--------|---------|---------|-------------|
| **host** | `host` | `distant` | Runs a distant server on the local/remote host via the `distant_host` crate |
| **ssh** | `ssh` | `ssh` | Pure Rust SSH client using `russh` via the `distant_ssh` crate |
| **docker** | `docker` | `docker` | Docker container interaction via the Bollard API (`distant_docker`) |

Built-in plugins implement the `Plugin` trait (`distant_core::Plugin`) directly.
They receive raw destination strings and handle URI parsing internally.



For the full API protocol reference (request/response types, capabilities, streaming
operations, per-plugin support matrix, and TCP tunneling protocol detail), see
[`docs/PROTOCOL.md`](PROTOCOL.md).

---

## Example: Minimal Plugin (Bash)

A minimal plugin that only supports `connect` (not `launch`):

```bash
#!/bin/bash
case "$1" in
  connect)
    DEST="$2"
    # ... perform connection setup ...
    echo '{"ready": true}'
    # Now proxy JSON-lines between stdin/stdout and the remote server
    ;;
  launch)
    echo '{"error": {"kind": "unsupported", "description": "launch not supported"}}'
    exit 1
    ;;
  *)
    echo "Usage: $0 {launch|connect} <destination> [options]" >&2
    exit 1
    ;;
esac
```

## Example: Auth Relay Round-Trip (Bash)

A plugin that prompts for a password before connecting:

```bash
#!/bin/bash
case "$1" in
  connect)
    DEST="$2"

    # Send an auth challenge to the manager
    echo '{"auth_challenge": {"questions": [{"text": "Password:", "options": {"echo": "false"}}]}}'

    # Read the auth response from stdin
    read -r RESPONSE
    PASSWORD=$(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['auth_response']['answers'][0])")

    # Use the password to authenticate (example: curl with basic auth)
    HOST=$(echo "$DEST" | sed 's|.*://||')
    if curl -sf -u "user:$PASSWORD" "http://$HOST/api/health" > /dev/null 2>&1; then
      echo '{"ready": true}'
      # Proxy loop: read requests from stdin, forward to server, write responses to stdout
      while read -r REQUEST; do
        RESPONSE=$(curl -sf -u "user:$PASSWORD" -X POST -d "$REQUEST" "http://$HOST/api/distant")
        echo "$RESPONSE"
      done
    else
      echo '{"error": {"kind": "permission_denied", "description": "Authentication failed"}}'
      exit 1
    fi
    ;;
  launch)
    echo '{"error": {"kind": "unsupported", "description": "launch not supported"}}'
    exit 1
    ;;
esac
```

## Troubleshooting

Common issues when developing plugins:

- **Plugin not found**: Ensure the `path` in `plugins.toml` is an absolute path and
  the binary is executable (`chmod +x`).
- **No response from plugin**: The manager expects JSON-lines on stdout. Ensure all
  debug output goes to stderr, not stdout.
- **Timeout errors**: The manager enforces a 120-second timeout on setup. If your
  plugin takes longer (e.g., pulling Docker images), consider optimizing the
  operation or sending periodic auth challenges to keep the connection alive.
- **Debugging**: Run distant with `--log-level debug` to see the full JSON-lines
  exchange between the manager and plugin:
  ```bash
  distant manager listen --log-level debug
  ```
