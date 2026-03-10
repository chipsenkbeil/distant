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

---

## Distant API Protocol Reference

After the setup phase completes, the manager exchanges **request** and **response** messages with the plugin as JSON-lines. Each request maps to one or more responses. Responses are either synchronous (one response per request) or streaming (multiple asynchronous responses over time).

### Capabilities

Plugins advertise supported operations via capability strings in their `Version` response. Clients can query capabilities to determine which operations are available before attempting them.

| Capability | Constant | Description |
|------------|----------|-------------|
| `tcp_tunnel` | `CAP_TCP_TUNNEL` | Forward TCP tunneling (server connects out) |
| `tcp_rev_tunnel` | `CAP_TCP_REV_TUNNEL` | Reverse TCP tunneling (server listens for incoming) |

### Request Types

#### File Operations

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `file_read` | `path` | `Blob` | Read file contents as bytes |
| `file_read_text` | `path` | `Text` | Read file contents as UTF-8 text |
| `file_write` | `path`, `data` | `Ok` | Write bytes to file (creates/overwrites) |
| `file_write_text` | `path`, `text` | `Ok` | Write UTF-8 text to file |
| `file_append` | `path`, `data` | `Ok` | Append bytes to file |
| `file_append_text` | `path`, `text` | `Ok` | Append UTF-8 text to file |

#### Directory Operations

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `dir_read` | `path`, `depth`, `absolute`, `canonicalize`, `include_root` | `DirEntries` | List directory contents |
| `dir_create` | `path`, `all` | `Ok` | Create directory (optionally recursive) |

#### Path Operations

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `remove` | `path`, `force` | `Ok` | Remove file or directory |
| `copy` | `src`, `dst` | `Ok` | Copy file or directory |
| `rename` | `src`, `dst` | `Ok` | Rename/move file or directory |
| `exists` | `path` | `Exists` | Check if path exists |
| `metadata` | `path`, `canonicalize`, `resolve_file_type` | `Metadata` | Get file/directory metadata |
| `set_permissions` | `path`, `permissions`, `options` | `Ok` | Set file permissions |

#### Watch Operations (Streaming)

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `watch` | `path`, `recursive`, `only`, `except` | `Ok` + streaming `Changed` | Watch path for filesystem changes |
| `unwatch` | `path` | `Ok` | Stop watching a path |

#### Search Operations (Streaming)

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `search` | `query` | `SearchStarted` + streaming `SearchResults` + `SearchDone` | Search files by content or path pattern |
| `cancel_search` | `id` | `Ok` | Cancel an active search |

#### Process Operations (Streaming)

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `proc_spawn` | `cmd`, `environment`, `current_dir`, `pty` | `ProcSpawned` + streaming `ProcStdout`/`ProcStderr`/`ProcDone` | Spawn a remote process |
| `proc_kill` | `id` | `Ok` | Kill a running process |
| `proc_stdin` | `id`, `data` | `Ok` | Write to a process's stdin |
| `proc_resize_pty` | `id`, `size` | `Ok` | Resize a process's PTY |

#### Tunnel Operations (Streaming)

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `tunnel_open` | `host`, `port` | `TunnelOpened` + streaming `TunnelData`/`TunnelClosed` | Open a forward TCP tunnel (server connects to host:port) |
| `tunnel_listen` | `host`, `port` | `TunnelListening` + streaming `TunnelIncoming`/`TunnelData`/`TunnelClosed` | Start a reverse TCP listener on the server |
| `tunnel_write` | `id`, `data` | `Ok` | Write data to an active tunnel |
| `tunnel_close` | `id` | `Ok` | Close a tunnel or listener |
| `tunnel_list` | _(empty)_ | `TunnelEntries` | List all active tunnels and listeners |

#### System Operations

| Request | Fields | Response | Description |
|---------|--------|----------|-------------|
| `system_info` | _(empty)_ | `SystemInfo` | Get remote system information |
| `version` | _(empty)_ | `Version` | Get server version and capabilities |

### Response Types

| Response | Fields | Description |
|----------|--------|-------------|
| `ok` | _(empty)_ | Success acknowledgement |
| `error` | `kind`, `description` | Error with kind and message |
| `blob` | `data` | Binary data (base64 in JSON) |
| `text` | `data` | UTF-8 text data |
| `dir_entries` | `entries`, `errors` | Directory listing |
| `exists` | `value` | Boolean existence check |
| `metadata` | _(various)_ | File/directory metadata |
| `changed` | _(various)_ | Filesystem change notification |
| `system_info` | _(various)_ | Remote system information |
| `version` | `server_version`, `protocol_version`, `capabilities` | Server version and capabilities |
| `search_started` | `id` | Search operation started |
| `search_results` | `id`, `matches` | Search matches (streamed) |
| `search_done` | `id` | Search operation complete |
| `proc_spawned` | `id` | Process started |
| `proc_stdout` | `id`, `data` | Process stdout data (streamed) |
| `proc_stderr` | `id`, `data` | Process stderr data (streamed) |
| `proc_done` | `id`, `success`, `code` | Process exited |
| `tunnel_opened` | `id` | Forward tunnel connected |
| `tunnel_listening` | `id`, `port` | Reverse listener bound (actual port) |
| `tunnel_data` | `id`, `data` | Data from tunnel (streamed) |
| `tunnel_incoming` | `listener_id`, `tunnel_id`, `peer_addr` | New connection on reverse listener |
| `tunnel_closed` | `id` | Tunnel or listener closed |
| `tunnel_entries` | `entries` | List of active tunnels |

### Streaming Operations

Several operations produce multiple responses over time. The plugin must continue sending streaming responses until the operation completes or is cancelled.

**Process I/O:** After `ProcSpawned`, the plugin streams `ProcStdout` and `ProcStderr` as data arrives. The client sends `ProcStdin` to write to the process. `ProcDone` signals process exit.

**Search:** After `SearchStarted`, the plugin streams `SearchResults` as matches are found. `SearchDone` signals search completion. `CancelSearch` stops the operation early.

**Watch:** After the initial `Ok`, the plugin streams `Changed` responses whenever the watched path changes. `Unwatch` stops the watch.

**Tunneling:** After `TunnelOpened` or `TunnelListening`, the plugin streams `TunnelData` as data arrives on the TCP connection. For reverse tunnels, `TunnelIncoming` is sent for each new connection. The client sends `TunnelWrite` to push data. `TunnelClosed` signals the end of a tunnel or listener.

---

## Per-Plugin Support Matrix

Not all plugins support every operation. The table below shows which operations are supported by each built-in plugin:

| Operation | host | ssh | docker |
|-----------|:----:|:---:|:------:|
| File read/write | Yes | Yes | Yes |
| Directory operations | Yes | Yes | Yes |
| Path operations | Yes | Yes | Yes |
| Watch | Yes | No | No |
| Search | Yes | Yes | Yes (best-effort) |
| Process spawn | Yes | Yes | Yes |
| Tunnel open (forward) | Yes | Yes | Yes (best-effort) |
| Tunnel listen (reverse) | Yes | No | No |
| System info | Yes | Yes | Yes |

**Notes:**
- **ssh** forward tunneling uses SSH direct-tcpip channels (`channel_open_direct_tcpip`). Reverse tunneling is not supported because russh's `tcpip_forward` requires mutable handle access incompatible with the Api trait.
- **docker** forward tunneling uses `socat` or `nc` inside the container via `docker exec`. Requires one of these tools to be installed in the container image. Reverse tunneling is not supported because Docker exec's single stdin/stdout pair cannot multiplex multiple incoming connections.
- **docker** search uses `rg`, `grep`, or `find` inside the container (best-effort, depends on available tools).

---

## TCP Tunneling Protocol Detail

TCP tunneling allows forwarding arbitrary TCP connections through a distant session. It supports two directions:

- **Forward** (`ssh -L` equivalent): The server connects to a remote host:port on behalf of the client.
- **Reverse** (`ssh -R` equivalent): The server listens on a port and relays incoming connections to the client.

### Forward Tunnel Flow

```
Client CLI              distant protocol           Server (host/ssh/docker)
------------------------------------------------------------------------
local TCP accepted  ->  TunnelOpen{host,port}    -> TcpStream::connect()
                    <-  TunnelOpened{id}          <-
local TCP data      ->  TunnelWrite{id,data}     -> tcp.write(data)
                    <-  TunnelData{id,data}       <- tcp.read() loop
local TCP close     ->  TunnelClose{id}          -> drop tcp
                    <-  TunnelClosed{id}          <- (or remote closes first)
```

1. Client sends `TunnelOpen` with the target host and port.
2. Server connects to the target via TCP and returns `TunnelOpened` with a tunnel ID.
3. Client sends data via `TunnelWrite`; server relays it to the TCP connection.
4. Server streams data back via `TunnelData` as it arrives from the TCP connection.
5. Either side can close: client sends `TunnelClose`, or server sends `TunnelClosed` when the TCP connection drops.

### Reverse Tunnel Flow

```
Client CLI              distant protocol           Server
------------------------------------------------------------------------
                    ->  TunnelListen{host,port}   -> TcpListener::bind()
                    <-  TunnelListening{id,port}  <-
                    <-  TunnelIncoming{lid,tid}   <- listener.accept()
local TCP connect
local TCP data      ->  TunnelWrite{tid,data}    -> tcp.write(data)
                    <-  TunnelData{tid,data}      <- tcp.read() loop
                    ->  TunnelClose{id}           -> drop listener + all subs
```

1. Client sends `TunnelListen` with the bind host and port (port 0 for OS-assigned).
2. Server binds a TCP listener and returns `TunnelListening` with the listener ID and actual port.
3. When a connection arrives, server sends `TunnelIncoming` with the listener ID, a new tunnel ID, and the peer address.
4. Data flows bidirectionally via `TunnelWrite` (client-to-server) and `TunnelData` (server-to-client).
5. Closing the listener ID closes the listener and all its sub-tunnels.

### SSH Launch Tunneling

SSH launch tunneling eliminates the need for open ports on the remote host by routing the distant protocol through an SSH channel:

```
Client                  SSH Channel                Remote
------------------------------------------------------------------------
ssh connect + auth  ->                           ->
exec "distant server listen --host 127.0.0.1"   -> server starts (localhost)
read credentials    <-                           <- stdout: host:port:key
channel_open_direct_tcpip(127.0.0.1, port)       ->
distant protocol frames over SSH channel          <-> (no open port needed)
```

Enable with `--tunnel` or `--ssh.tunnel=true`:
```
distant launch ssh://host --tunnel
```

The server binds to `127.0.0.1` instead of a public interface, and the client connects through an SSH direct-tcpip channel to `127.0.0.1:port`. No firewall rules or port exposure needed.

### Tunnel Identification

All tunnels (forward connections, reverse listeners, and reverse sub-connections) share a single ID space. Each ID is unique within a session. This allows `TunnelClose` to work uniformly — closing a listener ID also closes all its accepted sub-tunnels.

`TunnelList` / `TunnelEntries` returns all active tunnels with their direction, host, and port:

```json
{"tunnel_entries": {"entries": [
  {"id": 1, "direction": "forward", "host": "db-host", "port": 5432},
  {"id": 3, "direction": "reverse", "host": "0.0.0.0", "port": 9090}
]}}
```

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
