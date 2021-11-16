# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/chipsenkbeil/distant/compare/v0.15.1...HEAD
[0.15.1]: https://github.com/chipsenkbeil/distant/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/chipsenkbeil/distant/compare/v0.14.0...v0.15.0
