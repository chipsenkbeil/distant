# distant - remotely edit files and run programs

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![CI][distant_ci_img]][distant_ci_lnk] [![RustC 1.61+][distant_rustc_img]][distant_rustc_lnk] 

[distant_crates_img]: https://img.shields.io/crates/v/distant.svg
[distant_crates_lnk]: https://crates.io/crates/distant
[distant_doc_img]: https://docs.rs/distant/badge.svg
[distant_doc_lnk]: https://docs.rs/distant
[distant_ci_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml/badge.svg
[distant_ci_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml
[distant_rustc_img]: https://img.shields.io/badge/distant-rustc_1.61+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2022/05/19/Rust-1.61.0.html

🚧 **(Alpha stage software) This program is in rapid development and may break or change frequently!** 🚧

## Details

The `distant` binary supplies both a server and client component as well as
a command to start a server and configure the local client to be able to
talk to the server.

- Asynchronous in nature, powered by [`tokio`](https://tokio.rs/)
- Data is serialized to send across the wire via [`msgpack`](https://msgpack.org/)
- Encryption & authentication are handled via
  [XChaCha20Poly1305](https://tools.ietf.org/html/rfc8439) for an authenticated
  encryption scheme via
  [RustCrypto/ChaCha20Poly1305](https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305)

Additionally, the core of the distant client and server codebase can be pulled
in to be used with your own Rust crates via the `distant-core` crate. The
networking library, which is agnostic of `distant` protocols, can be used via
the `distant-net` crate.

## Installation

### Prebuilt Binaries

If you would like a pre-built binary, check out the 
[releases section](https://github.com/chipsenkbeil/distant/releases).

### Building from Source

If you have [`cargo`](https://github.com/rust-lang/cargo) installed, you can
directly download and build the source via:

```bash
cargo install distant
```

Alternatively, you can clone this repository and build from source following
the [build guide](./BUILDING.md).

## Backend Feature Matrix

Distant supports multiple backends to facilitate remote communication with
another server. Today, these backends include:

* `distant` - a standalone server acting as the reference implementation
* `ssh` - a wrapper around an `ssh` client that translates the distant protocol
  into ssh server requests

Not every backend supports every feature of distant. Below is a table outlining
the available features and which backend supports each feature:

| Feature               | distant | ssh |
| --------------------- | --------| ----|
| Capabilities          | ✅      | ✅  |
| Filesystem I/O        | ✅      | ✅  |
| Filesystem Watching   | ✅      | ✅  |
| Process Execution     | ✅      | ✅  |
| Reconnect             | ✅      | ❌  |
| Search                | ✅      | ❌  |
| System Information    | ✅      | ⚠  |

* ✅ means full support
* ⚠ means partial support
* ❌ means no support

### Feature Details

* `Capabilities` - able to report back what it is capable of performing
* `Filesystem I/O` - able to read from and write to the filesystem
* `Filesystem Watching` - able to receive notifications when changes to the
  filesystem occur
* `Process Execution` - able to execute processes
* `Reconnect` - able to reconnect after network outages
* `Search` - able to search the filesystem
* `System Information` - able to retrieve information about the system

## Example

### Starting the manager

In order to facilitate communication between a client and server, you first
need to start the manager. This can be done in one of two ways:

1. Leverage the `service` functionality to spawn the manager using one of the
   following supported service management platforms:
  - [`sc.exe`](https://docs.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2012-r2-and-2012/cc754599(v=ws.11)) for use with [Window Service](https://en.wikipedia.org/wiki/Windows_service) (Windows)
  - [Launchd](https://en.wikipedia.org/wiki/Launchd) (MacOS)
  - [systemd](https://en.wikipedia.org/wiki/Systemd) (Linux)
  - [OpenRC](https://en.wikipedia.org/wiki/OpenRC) (Linux)
  - [rc.d](https://en.wikipedia.org/wiki/Init#Research_Unix-style/BSD-style) (FreeBSD)
2. Run the manager manually by using the `listen` subcommand

#### Service management

```bash
# If you want to install the manager as a service, you can use the service
# interface available directly from the CLI
#
# By default, this will install a system-level service, which means that you
# will need elevated permissions to both install AND communicate with the
# manager
distant manager service install

# If you want to maintain a user-level manager service, you can include the
# --user flag. Note that this is only supported on MacOS (via launchd) and
# Linux (via systemd)
distant manager service install --user

# ........

# Once you have installed the service, you will normally need to start it
# manually or restart your machine to trigger startup on boot
distant manager service start # --user if you are working with user-level
```

#### Manual start

```bash
# If you choose to run the manager without a service management platform, you
# can either run the manager in the foreground or provide --daemon to spawn and
# detach the manager

# Run in the foreground
distant manager listen

# Detach the manager where it will not terminate even if the parent exits
distant manager listen --daemon
```

### Interacting with a remote machine

Once you have a manager listening for client requests, you can begin
interacting with the manager, spawn and/or connect to servers, and interact
with remote machines.

```bash
# Connect to my.example.com on port 22 via SSH and start a distant server
distant client launch ssh://my.example.com

# After the connection is established, you can perform different operations
# on the remote machine via `distant client action {command} [args]`
distant client action copy path/to/file new/path/to/file
distant client action spawn -- echo 'Hello, this is from the other side'

# Opening a shell to the remote machine is trivial
distant client shell

# If you have more than one connection open, you can switch between active
# connections by using the `select` subcommand
distant client select '<ID>'

# For programmatic use, a REPL following the JSON API is available
distant client repl --format json
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
