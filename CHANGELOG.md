# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `SystemInfo` via ssh backend now detects and reports username and shell
- `SystemInfo` via ssh backend now reports os when windows detected
- `Capabilities` request/response for server and manager that report back the
  capabilities (and descriptions) supported by the server or manager
- `Search` and `CancelSearch` request/response for server that performs a
  search using `grep` crate against paths or file contents, returning results
  back as a stream
  - New `Searcher` available as part of distant client interface to support
    performing a search and getting back results
  - Updated `DistantChannelExt` to support creating a `Searcher` and canceling
    an ongoing search query
  - `distant client action search` now supported, waiting for results and
    printing them out

### Changed

- `SystemInfo` data type now includes two additional fields: `username` and
  `shell`. The `username` field represents the name of the user running the
  server process. The `shell` field points to the default shell associated with
  the user running the server process

### Fixed

- `distant client shell` will now use the default shell from system info, or
  choose between `/bin/sh` and `cmd.exe` as the default shell based on the
  family returned by a system info request
- `distant client shell` properly terminates master pty when the shell exits,
  resolving the hanging that occurred for Windows `cmd.exe` and
  `powershell.exe` upon exit
- ssh launch with login shell now only uses `sh` when remote family is `unix`
- ssh backend implementation of copy now works more widely across windows
  systems by switching to `powershell.exe` to perform copy

## [0.18.0] - 2022-08-18
### Changed

- `shutdown-after` replaced with `shutdown` that supports three options:
  1. `never` - server will never shutdown automatically
  2. `after=N` - server will shutdown after N seconds
  3. `lonely=N` - server will shutdown N seconds after no connections

## [0.17.6] - 2022-08-18
### Fixed

- `shutdown-after` cli parameter and config option now properly shuts down
  server after N seconds with no connections

## [0.17.5] - 2022-08-18
### Fixed

- Handle `RecommendedWatcher` failing with an unsupported OS function on M1 Mac
  architecture running a Linux container via Docker 
  ([notify #423](https://github.com/notify-rs/notify/issues/423))

## [0.17.4] - 2022-08-18
### Fixed

- Parsing of a host for `Destination` now correctly handles IPv6 addresses such
  that `::1` and `[::1]:12345` are captured into host and port
- Displaying of `Distant` and `DistantSingleKeyCredentials` now properly wrap
  IPv6 addresses in square brackets when a port is available

## [0.17.3] - 2022-08-18
### Added

- New `ClientConnectConfig` to support connect settings, specifically for ssh
- `Host` with `HostParseError` that follows the 
  [DoD Internet Host Table Specification](https://www.ietf.org/rfc/rfc0952.txt)
  and subsequent [RFC-1123](https://www.rfc-editor.org/rfc/rfc1123)

### Changed

- `Destination` now has direct fields for scheme, username, password, host, and
  port that are populated from parsing
- `Destination` no longer wraps `uriparse::URI` and all references to
  implementing/wrapping have been removed

### Fixed

- `ssh` option to specify external binary not working on `launch` due to the
  key being mis-labeled as `ssh.bind` instead of `ssh.bin`
- All ssh settings were not being applied with manager handlers due to some key
  checks being incorrect (e.g. `backend` instead of `ssh.backend`). This has
  now been corrected and settings now properly get applied

### Removed

- The ssh settings of `ssh.user` and `ssh.port` were unused as these were now
  being taking from the destination `ssh://[username:]host[:port]`, so they
  have now been removed to avoid confusion
- Remove `uriparse` dependency

## [0.17.2] - 2022-08-16
### Added

- `replace_scheme` method to `Destination`

### Fixed

- `DistantManagerRouter` no longer silently fails when `distant.args` is
  provided that includes double quotes within it

### Changed

- `Map` implementation of `Display` now escapes `\` and `"`
- `Map` implementation of `FromStr` now handles escaped `\` and `"`
- Split `fallback_scheme` for `DistantManagerConfig` into
  `launch_fallback_scheme` (defaulting to ssh) and `connect_fallback_scheme`
  (defaulting to distant); this means that subsequent use of 
  `distant client launch [user@]host[:port]` will now default to ssh instead of
  failing due to lack of distant launch handler
- Expose `windows-pipe` and `unix-socket` config and cli options regardless of
  platform (so they can be provided without worrying about which OS)
- Lock `--access` to `distant manager listen` as a cli parameter and move it
  out of `[network]` config to be tied to manager config only

## [0.17.1] - 2022-08-16
### Added

- New `format` option available for `client select`
  - Choices are provided via `{"type": "select", "choices": ["...", ...], "current": 0}`
  - Selection is specified via `{"type": "selected", "choice": 0}`

### Fixed

- `distant client launch` using `--format json` now properly prints out id in
  JSON format (`{"type": "launched", "id": "..."}`)
- `distant client connect` using `--format json` now properly prints out id in
  JSON format (`{"type": "connected", "id": "..."}`)

## [0.17.0] - 2022-08-09
### Added

- `distant manager` subcommand
  - `distant manager service` subcommand contains functionality to install,
    start, stop, and uninstall the manager as a service on various operating
    systems
  - `distant manager info` will print information about an active connection
  - `distant manager list` will print information about all connections
- `distant generate` subcommand
  - `distant generate schema` will produce JSON schema for server
    request/response
  - `distant generate completion` will produce completion file for a specific
    shell

### Changed

- `distant launch` is now `distant client launch`
- `distant action` is now `distant client action`
- `distant shell` is now `distant client shell`
- `distant listen` is now `distant server listen`
- Minimum supported rust version (MSRV) has been bumped to `1.61.0`

### Fixed

- Shell no longer has issues with fancier command prompts and other
  terminal-oriented printing as `TERM=x256-color` is now set by default

### Removed

- Networking directly from distant client to distant server. All connections
  are now facilitated by the manager interface with client -> manager -> server
- Environment variable output as part of launch is now gone as the connection
  is now being managed, so there is no need to export session information

## [0.16.4] - 2022-06-01
### Added
- Dockerfile using Alpine linux with a basic install of distant, tagged as
  `chipsenkbeil/distant:0.16.3` and `chipsenkbeil/distant:0.16.4`

### Fixed
- [Issue #90](https://github.com/chipsenkbeil/distant/issues/90)
- [Issue #103](https://github.com/chipsenkbeil/distant/issues/103)

## [0.16.3] - 2022-05-29
### Added
- New `--ssh-backend` option for CLI that accepts `libssh` or `ssh2` for
  native backend ssh support
- `distant_ssh2::SshBackend` now supports parsing from a `&str` and producing a
  `&'static str` from an instance

## [0.16.2] - 2022-05-27
### Changed
- The following fields now default to false when missing in JSON request body
  - For `DirRead`: `absolute`, `canonicalize`, `include_root`
  - For `DirCreate`: `all`
  - For `Remove`: `force`
  - For `Watch`: `recursive`
  - For `Metadata`: `canonicalize` and `resolve_file_type`
  - For `ProcSpawn`: `args` (empty list), `persist`, and `pty` (nothing)

## [0.16.1] - 2022-05-13
### Changed
- Lock in to Rust openssl 0.10.38 as it is the last version that supports using
  openssl 3.x.x before reverting

## [0.16.0] - 2022-05-12
### Added
- New `environment` session type that prints out environment variable
  definitions for use in an interactive session or to evaluate
- Shell support introduced for ssh & distant servers, including a new shell
  command for distant cli
- Support for JSON communication of ssh auth during launch (cli)
- Add windows and unix metadata files to overall metadata response data
- Watch and unwatch cli commands powered by underlying `Watcher` core
  implementation that uses new `RequestData::Watch`, `RequestData::Unwatch`,
  and `ResponseData::Changed` data types to communicate filesystem changes

### Changed
- Default session type for CLI (launch, action, etc) is `environment`
- Replace cbor library with alternative as old cbor lib has been abandoned
- Refactor some request & response types to work with new cbor lib
- Updated cli to always include serde dependency
- Expose `origin_id` of remote process as method
- Rename ProcRun -> ProcSpawn, ProcStarted -> ProcSpawned
- Update ProcStdin, ProcStdout, and ProcStderr to use list of bytes instead
  of a string as a parameter; RemoteProcess and RemoteLspProcess now support
  reading and writing using either `String` or `Vec<u8>`
- Rename `--detached` and associated to `--persist`

### Removed
- Github actions no longer use paths-filter so every PR & commit will test
  everything
- `distant-lua` and `distant-lua-test` no longer exist as we are focusing
  solely on the JSON API for integration into distant

## [0.15.1] - 2021-11-15
### Added
- `--key-from-stdin` option to listen cli command to read key from stdin
  instead of generating
- More logging to distant and ssh handler *proc run* methods
- Disclaimer to distant session about dns resolution

### Changed
- Improve the distant-core readme

### Removed
- DNS resolution for ssh session 

## [0.15.0] - 2021-10-16
### Added
- distant-ssh2 subcrate to provide an alternate session as an ssh client
- distant-lua subcrate for lua lib 
- `rpassword` & `wezterm-ssh` dependencies for distant-ssh2 and
  `XChaCha20Poly1305` dependency in place of `orion` for encryption
- `Codec` trait to support encode & decode
- `XChaCha20Poly1305Codec` that encrypts/signs using *XChaCha20Poly1305*
- `PlainCodec` that does no encrypting/signing
- `SessionChannelExt` trait for friendlier methods
- `Mailbox` and internal `PostOffice` to manage responses to requests
- Method parameter to support distant & ssh methods for action and lsp subcommands
- Support compiling distant cli on windows (#59)
- `status` method to `RemoteProcess`

### Changed
- Refactor Transport to take generic Codec
- Rewrite to no longer use blake256
- Refactor `Session` to use a new `SessionChannel` underneath
- Refactor `Response` to always include an *origin_id* field instead of being
  optional
- Update `ProcStdout`, `ProcStderr`, and `ProcDone` to include origin id
- Replace `verbose` option with `log-level`
- Replace `DISTANT_AUTH_KEY` with `DISTANT_KEY` for environment variable parsing
- Refactor to support Minimum Supported Rust Version (MSRV) of 1.51.0
- Rename core -> distant-core in project directory structure
- Upgrade tokio to 1.12
- Update `Metadata` to be 
    - cloneable
    - debuggable
    - serializable
    - deserializable
- Refactor `Metadata` and `SystemInfo` response data types to support
  subtypes as singular parameters
- Replace `--daemon` in favor of opposite parameter `--foreground`

### Removed
- `DistantCodec`
- `k256` dependency
- `Transport::from_handshake` as no longer doing *EDCH key exchange*

### Fixed
- Stdout/stderr being sent before *proc_start* by adding *post_hook* support
  to handler such that *proc_run* tasks are not spawned until *proc_start* is
  sent as response
- `InmemoryStreamWriteHalf` implementation of AsyncWrite now properly yields
  pending upon full channel and no longer locks up
- stdout, stderr, and stdin of `RemoteProcess` no longer cause deadlock

[Unreleased]: https://github.com/chipsenkbeil/distant/compare/v0.17.6...HEAD
[0.17.6]: https://github.com/chipsenkbeil/distant/compare/v0.17.5...v0.17.6
[0.17.5]: https://github.com/chipsenkbeil/distant/compare/v0.17.4...v0.17.5
[0.17.4]: https://github.com/chipsenkbeil/distant/compare/v0.17.3...v0.17.4
[0.17.3]: https://github.com/chipsenkbeil/distant/compare/v0.17.2...v0.17.3
[0.17.2]: https://github.com/chipsenkbeil/distant/compare/v0.17.1...v0.17.2
[0.17.1]: https://github.com/chipsenkbeil/distant/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/chipsenkbeil/distant/compare/v0.16.4...v0.17.0
[0.16.4]: https://github.com/chipsenkbeil/distant/compare/v0.16.3...v0.16.4
[0.16.3]: https://github.com/chipsenkbeil/distant/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/chipsenkbeil/distant/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/chipsenkbeil/distant/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/chipsenkbeil/distant/compare/v0.15.1...v0.16.0
[0.15.1]: https://github.com/chipsenkbeil/distant/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/chipsenkbeil/distant/compare/v0.14.0...v0.15.0
