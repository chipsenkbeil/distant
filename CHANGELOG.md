# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Request` and `Response` types from `distant-net` now support an optional
  `Header` to send miscellaneous information
- New feature `tracing` provides https://github.com/tokio-rs/tracing support
  as a new `--tracing` flag. Must be compiled with
  `RUSTFLAGS="--cfg tokio_unstable"` to properly operate.

### Changed

- `Change` structure now provides a single `path` instead of `paths` with the
  `distant-local` implementation sending a separate `Changed` event per path
- `ChangeDetails` now includes a `renamed` field to capture the new path name
  when known
- `DistantApi` now handles batch requests in parallel, returning the results in
  order. To achieve the previous sequential processing of batch requests, the
  header value `sequence` needs to be set to true
- Rename `GenericServerRef` to `ServerRef` and remove `ServerRef` trait,
  refactoring `TcpServerRef`, `UnixSocketServerRef`, and `WindowsPipeServerRef`
  to use the struct instead of `Box<dyn ServerRef>`
- Update `Reply` trait and associated implementations to be non-blocking &
  synchronous as opposed to asynchronous to avoid deadlocks and also be more
  performant

## [0.20.0-alpha.8]

### Added

- `distant-local` now has two features: `macos-fsevent` and `macos-kqueue`.
  These are used to indicate what kind of file watching to support (for MacOS).
  The default is `macos-fsevent`.
- `[server.watch]` configuration is now available with the following
  settings:
  - `native = <bool>` to specify whether to use native watching or polling
    (default true)
  - `poll_interval = <secs>` to specify seconds to wait between polling
    attempts (only for polling watcher)
  - `compare_contents = <bool>` to specify how polling watcher will evaluate a
    file change (default false)
  - `debounce_timeout = <secs>` to specify how long to wait before sending a
    change notification (will aggregate and merge changes)
  - `debounce_tick_rate = <secs>` to specify how long to wait between event
    aggregation loops
- `distant-protocol` response for a change now supports these additional
  fields:
  - `timestamp` (serialized as `ts`) to communicate the seconds since unix
    epoch when the event was received
  - `details` containing `attributes` (clarify changes on attribute kind) and
    `extra` (to convey arbitrary platform-specific extra information)

### Changed

- Bump minimum Rust version to 1.68.0

### Removed

- `crossbeam-channel` dependency removed from notify by disabling its feature
  in order to avoid a `tokio::spawn` issue (https://github.com/notify-rs/notify/issues/380)

### Fixed

- usernames with `-` (hyphen) we're rejected as invalid

## [0.20.0-alpha.7]

### Added

- New `SetPermissions` enum variant on protocol request
- New `set_permissions` method available `DistantApi` and implemented by local
  server (ssh unavailable due to https://github.com/wez/wezterm/issues/3784)
- Implementation of `DistantChannelExt::set_permissions`
- `distant version` to display information about connected server
- `distant manager service install` now accepts additional arguments to provide
  the manager on startup

### Changed

- CLI `--lsp [<SCHEME>]` scheme now expects just the scheme and not `://`
- Moved `distant_net::common::authentication` to separate crate `distant-auth`
- Moved `distant_net::common::authentication::Keychain` to
  `distant_net::common::Keychain`
- Moved `distant_net::common::transport::framed::codec::encryption::SecretKey`
  and similar to `distant_net::common::SecretKey`
- Search matches reported with `match` key are now inlined as either a byte
  array or a string and no longer an object with a `type` and `value` field
- Unset options and values are not now returned in `JSON` serialization versus
  the explicit `null` value provided
- `Capabilities` message type has been changed to `Version` with new struct to
  report the version information that includes a server version string,
  protocol version tuple, and capabilities
- `distant_core::api::local` moved to `distant_local`

### Removed

- `distant capabilities` has been removed in favor of `distant version`

## [0.20.0-alpha.6]

### Changed

- Renamed `distant_core::data` to `distant_core::protocol`
- CLI `--lsp` now accepts an optional `scheme` to be used instead of
  `distant://`, which is the default
- `RemoteLspProcess` now takes a second argument, `scheme`, which dictates
  whether to translate `distant://` or something else

## [0.20.0-alpha.5]

### Added

- CLI now offers the following new subcommands
  - `distant fs copy` is a refactoring of `distant client action copy`
  - `distant fs exists` is a refactoring of `distant client action exists`
  - `distant fs read` is a refactoring of `distant client action file-read`,
    `distant client action file-read-text`, and `distant client action dir-read`
  - `distant fs rename` is a refactoring of `distant client action rename`
  - `distant fs write` is a refactoring of `distant client action file-write`,
    `distant client action file-write-text`, `distant client action file-append`,
  - `distant fs make-dir` is a refactoring of `distant client action dir-create`
  - `distant fs metadata` is a refactoring of `distant client action metadata`
  - `distant fs remove` is a refactoring of `distant client action remove`
  - `distant fs search` is a refactoring of `distant client action search`
  - `distant fs watch` is a refactoring of `distant client action watch`
  - `distant spawn` is a refactoring of `distant client action proc-spawn`
    with `distant client lsp` merged in using the `--lsp` flag
  - `distant system-info` is a refactoring of `distant client action system-info`
- Search now supports `upward` as a directional setting to traverse upward
  looking for results rather than recursing downward

### Changed

- CLI subcommands refactored
  - `distant client select` moved to `distant manager select`
  - `distant client action` moved to `distant action`
  - `distant client launch` moved to `distant launch`
  - `distant client connect` moved to `distant connect`
  - `distant client lsp` moved to `distant lsp`
  - `distant client repl` moved to `distant api`
  - `distant client shell` moved to `distant shell`

### Removed

- `distant-core` crate no longer offers the `clap` feature

### Fixed

- `distant launch manager://localhost` now rejects a bind address of `ssh`
  as the `SSH_CONNECTION` environment variable isn't available in most cases

## [0.20.0-alpha.4] - 2023-03-31

### Added

- Default configuration for `config.toml`
- Ability to generate default configuration using
  `distant generate config /path/to/config.toml`
- `--current-dir` option for `distant client shell` and `distant client lsp`

### Changed

- Updated a variety of dependencies to latest versions

## [0.20.0-alpha.3] - 2022-11-27

### Added

- `Frame::empty` method as convenience for `Frame::new(&[])`
- `ClientConfig` to support `ReconnectStrategy` and a duration serving as the
  maximum time to wait between server activity before attempting to reconnect
  from the client
- Server sends empty frames periodically to act as heartbeats to let the client
  know if the connection is still established
- Client now tracks length of time since last server activity and will attempt
  a reconnect if no activity beyond that point

### Changed

- `Frame` methods `read` and `write` no longer return an `io::Result<...>`
  and instead return `Option<Frame<...>>` and nothing respectively
- `Frame::read` method now supports zero-size items
- `Client::inmemory_spawn` and `UntypedClient::inmemory_spawn` now take a
  `ClientConfig` as the second argument instead of `ReconnectStrategy`
- Persist option now removed from `ProcSpawn` message and CLI
- Bump minimum Rust version to 1.64.0

### Removed

- `--no-shell` option is removed as we automatically detect and use the PTY of
  the remote system using a default shell

## [0.20.0-alpha.2] - 2022-11-20

### Added

- New `ConnectionState` and `ConnectionWatcher` to support watching changes to
  the client connection, supporting `clone_connection_watcher` and
  `on_connection_change` methods for the client

### Changed

- Server will now drop the connection if it receives an error (other than
  WouldBlock) while trying to read from the transport, rather than just logging
  the error, regardless of whether the error is resumable 

## [0.20.0-alpha.1] - 2022-11-19

**NOTE: This is incomplete as v0.20.0 is a near-complete rewrite internally.**

### Added

- New `contains` and `or` types for `SearchQueryCondition`

### Changed

- `SearchQueryCondition` now escapes regex for all types except `regex`
- Removed `min_depth` option from search
- Updated search to properly use binary detection, filter out common ignore
  file patterns, and execute in parallel via the `ignore` crate and `num_cpus`
  crate to calculate thread count

### Fixed

- Resolution of `BindAddress` now properly handles hostnames ranging from
  `localhost` to `example.com`
- Parsing of `BindAddress` no longer causes a stack overflow

## [0.19.0] - 2022-08-30
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

[Unreleased]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.8...HEAD
[0.20.0-alpha.8]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.7...v0.20.0-alpha.8
[0.20.0-alpha.7]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.6...v0.20.0-alpha.7
[0.20.0-alpha.6]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.5...v0.20.0-alpha.6
[0.20.0-alpha.5]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.4...v0.20.0-alpha.5
[0.20.0-alpha.4]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.3...v0.20.0-alpha.4
[0.20.0-alpha.3]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.2...v0.20.0-alpha.3
[0.20.0-alpha.2]: https://github.com/chipsenkbeil/distant/compare/v0.20.0-alpha.1...v0.20.0-alpha.2
[0.20.0-alpha.1]: https://github.com/chipsenkbeil/distant/compare/v0.19.0...v0.20.0-alpha.1
[0.19.0]: https://github.com/chipsenkbeil/distant/compare/v0.18.0...v0.19.0
[0.19.0]: https://github.com/chipsenkbeil/distant/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/chipsenkbeil/distant/compare/v0.17.6...v0.18.0
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
