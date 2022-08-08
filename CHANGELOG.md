# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
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

[Unreleased]: https://github.com/chipsenkbeil/distant/compare/v0.17.0...HEAD
[0.17.0]: https://github.com/chipsenkbeil/distant/compare/v0.16.4...v0.17.0
[0.16.4]: https://github.com/chipsenkbeil/distant/compare/v0.16.3...v0.16.4
[0.16.3]: https://github.com/chipsenkbeil/distant/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/chipsenkbeil/distant/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/chipsenkbeil/distant/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/chipsenkbeil/distant/compare/v0.15.1...v0.16.0
[0.15.1]: https://github.com/chipsenkbeil/distant/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/chipsenkbeil/distant/compare/v0.14.0...v0.15.0
