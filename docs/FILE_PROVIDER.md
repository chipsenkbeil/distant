# macOS File Provider

The macOS File Provider integration exposes remote filesystems mounted via
`distant connect` as native Finder locations. Users see a sidebar entry in
Finder and can browse, open, edit, and save files as if they were local.

## Prerequisites

- macOS 12+
- The `Distant.app` bundle must be installed (see [Building](#building))
- A distant manager daemon must be running
- An active connection to a remote server

## Quick Start

```bash
# 1. Start the manager daemon
/Applications/Distant.app/Contents/MacOS/distant manager listen --daemon

# 2. Connect to a remote server
/Applications/Distant.app/Contents/MacOS/distant connect ssh://user@host

# 3. Mount the remote filesystem in Finder
/Applications/Distant.app/Contents/MacOS/distant mount

# The mount appears in Finder's sidebar as "Distant — ssh-user@host"
```

## Building

The File Provider requires a signed `.app` bundle with an embedded `.appex`
extension. Use the provided build script:

```bash
scripts/make-app.sh
```

This builds the binary with File Provider support, creates the app bundle
structure, signs it, and installs to `/Applications/Distant.app`.

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CODESIGN_IDENTITY` | Auto-detect | Code signing identity |
| `APP_PROFILE` | (none) | App provisioning profile |
| `APPEX_PROFILE` | (none) | Appex provisioning profile |
| `CARGO_FEATURES` | `docker,host,ssh,...` | Cargo features to build |
| `INSTALL_DIR` | `/Applications` | Installation directory |

## Commands

### Mount

```bash
distant mount [--connection ID] [--remote-root PATH] [--readonly]
```

Registers a FileProvider domain with macOS. The mount appears in Finder's
sidebar immediately. No mount point directory is needed (unlike FUSE/NFS).

Options:
- `--connection ID` — use a specific connection (default: active connection)
- `--remote-root PATH` — expose a subdirectory instead of the server root
- `--readonly` — mount as read-only
- `--backend macos-file-provider` — explicitly select the File Provider backend
  (default on macOS when running inside the app bundle)

### Unmount

```bash
# Unmount by destination
distant unmount ssh://user@host

# Unmount all distant mounts
distant unmount --all
```

### Mount Status

```bash
# Show registered FileProvider domains
distant mount-status

# JSON output
distant mount-status --format json
```

Displays domain identifier, display name, metadata file presence, and
destination for each registered domain.

## Architecture

```
Finder --> fileproviderd --> DistantFileProvider.appex
                                   |
                             Unix socket (App Group)
                                   |
                             distant manager daemon
                                   |
                             SSH/Docker/etc channel
                                   |
                             remote server
```

The same `distant` binary serves both CLI and `.appex` roles. When launched
inside the app bundle's `PlugIns/` directory, it detects the `.appex` context
and enters extension mode instead of parsing CLI arguments.

### IPC

The `.appex` extension communicates with the manager daemon via a Unix socket
in the App Group shared container (`39C6AGD73Z.group.dev.distant`). Domain
metadata (connection ID, destination) is persisted as JSON files in the
`domains/` subdirectory.

### Caching

Three levels of caching reduce round-trips to the remote server:

| Cache | Default TTL | Purpose |
|-------|-------------|---------|
| Attribute | 1s | File metadata (size, mtime, type) |
| Directory | 1s | Directory listings |
| Read | 1s | File content |

TTLs can be adjusted via `--attr-ttl` and `--dir-ttl` on the mount command.

## Diagnostics

### Viewing Logs

```bash
# Show last 5 minutes of appex logs
scripts/logs-appex.sh

# Show last 30 minutes
scripts/logs-appex.sh --minutes 30

# Stream live logs
scripts/logs-appex.sh --follow

# List recent crash reports
scripts/logs-appex.sh --crashes
```

Log files are stored in:
- `~/Library/Group Containers/39C6AGD73Z.group.dev.distant/logs/`
- Fallback: `/tmp/distant-appex-{pid}.log`

### System Logs

```bash
# View fileproviderd logs
log show --predicate 'process == "distant"' --last 5m --style compact

# Stream live
log stream --predicate 'process == "distant"' --style compact
```

### Common Issues

**"Loading..." forever in Finder**

The extension may have failed to bootstrap. Check logs for bootstrap errors:
```bash
scripts/logs-appex.sh | grep -i bootstrap
```

Common causes:
- Manager daemon not running (`distant manager listen --daemon`)
- Connection expired (re-run `distant connect`)
- Metadata file missing (check `distant mount-status`)

**Mount not visible in Finder sidebar**

FileProvider domains appear under "Locations" in Finder's sidebar. If missing:

1. **Enable the extension**: System Settings > General > Login Items &
   Extensions > Added Extensions. Find "Distant" and enable it. The build
   script runs `pluginkit -e use` automatically, but macOS may still require
   manual confirmation.
2. **Check Finder preferences**: Finder > Settings (Cmd+,) > Sidebar tab.
   Ensure cloud storage locations are enabled under "Locations".
3. **Restart fileproviderd**: `sudo killall fileproviderd` (it auto-restarts).
4. Mounts are always accessible at `~/Library/CloudStorage/` even if the
   sidebar entry doesn't appear.

**Extension crashes**

Check crash reports:
```bash
scripts/logs-appex.sh --crashes
```

## Limitations

- **No offline mode**: Files are fetched on demand, not pre-synced.
- **Single-writer**: Last write wins; no conflict resolution.
- **No thumbnails/Quick Look**: Not yet implemented.
- **Large files**: Loaded entirely into memory for transfer.
- **macOS only**: Not available on iOS/iPadOS.
