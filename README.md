# distant - remotely edit files and run programs

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![RustC 1.51+][distant_rustc_img]][distant_rustc_lnk] 

| Operating System | Status                                                             |
| ---------------- | ------------------------------------------------------------------ |
| MacOS (x86, ARM) | [![MacOS CI][distant_ci_macos_img]][distant_ci_macos_lnk]          |
| Linux (x86)      | [![Linux CI][distant_ci_linux_img]][distant_ci_linux_lnk]          |
| Windows (x86)    | [![Windows CI][distant_ci_windows_img]][distant_ci_windows_lnk]    |

[distant_crates_img]: https://img.shields.io/crates/v/distant.svg
[distant_crates_lnk]: https://crates.io/crates/distant
[distant_doc_img]: https://docs.rs/distant/badge.svg
[distant_doc_lnk]: https://docs.rs/distant
[distant_rustc_img]: https://img.shields.io/badge/distant-rustc_1.51+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2021/03/25/Rust-1.51.0.html

[distant_ci_macos_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-macos.yml/badge.svg
[distant_ci_macos_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-macos.yml
[distant_ci_linux_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-linux.yml/badge.svg
[distant_ci_linux_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-linux.yml
[distant_ci_windows_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-windows.yml/badge.svg
[distant_ci_windows_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci-windows.yml

🚧 **(Alpha stage software) This program is in rapid development and may break or change frequently!** 🚧

## Details

The `distant` binary supplies both a server and client component as well as
a command to start a server and configure the local client to be able to
talk to the server.

- Asynchronous in nature, powered by [`tokio`](https://tokio.rs/)
- Data is serialized to send across the wire via [`CBOR`](https://cbor.io/)
- Encryption & authentication are handled via
  [XChaCha20Poly1305](https://tools.ietf.org/html/rfc8439) for an authenticated
  encryption scheme via
  [RustCrypto/ChaCha20Poly1305](https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305)

Additionally, the core of the distant client and server codebase can be pulled
in to be used with your own Rust crates via the `distant-core` crate.
Separately, Lua bindings can be found within `distant-lua`, exported as a
shared library that can be imported into lua using `require("distant_lua")`.

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

## Examples

Launch a remote instance of `distant`. Calling `launch` will do the following:

1. Ssh into the specified host (in the below example, `my.example.com`)
2. Execute `distant listen --host ssh` on the remote machine
3. Receive on the local machine the credentials needed to connect to the server
4. Depending on the options specified, print/store/use the session settings so
   future calls to `distant action` can connect

```bash
# Connects to my.example.com on port 22 via SSH to start a new session
# and print out information to configure your system to talk to it
distant launch my.example.com

# NOTE: If you are using sh, bash, or zsh, you can automatically set the
        appropriate environment variables using the following
eval "$(distant launch my.example.com)"

# After the session is established, you can perform different operations
# on the remote machine via `distant action {command} [args]`
distant action copy path/to/file new/path/to/file
distant action spawn -- echo 'Hello, this is from the other side'
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
