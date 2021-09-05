# distant

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![CI][distant_ci_img]][distant_ci_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant.svg
[distant_crates_lnk]: https://crates.io/crates/distant
[distant_doc_img]: https://docs.rs/distant/badge.svg
[distant_doc_lnk]: https://docs.rs/distant
[distant_ci_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml/badge.svg
[distant_ci_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml

Binary to connect with a remote machine to edit files and run programs.

🚧 **(Alpha stage software) This program is in rapid development and may break or change frequently!** 🚧

## Details

The `distant` binary supplies both a server and client component as well as
a command to start a server and configure the local client to be able to
talk to the server.

- Asynchronous in nature, powered by [`tokio`](https://tokio.rs/)
- Data is serialized to send across the wire via [`CBOR`](https://cbor.io/)
- Encryption & authentication are handled via [`orion`](https://crates.io/crates/orion)
    - [XChaCha20Poly1305](https://cryptopp.com/wiki/XChaCha20Poly1305) for an authenticated encryption scheme
    - [BLAKE2b-256](https://www.blake2.net/) in keyed mode for a second authentication
    - [Elliptic Curve Diffie-Hellman](https://en.wikipedia.org/wiki/Elliptic-curve_Diffie%E2%80%93Hellman) (ECDH) for key exchange

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

Launch a remote instance of `distant` by SSHing into another machine and
starting the `distant` executable:

```bash
# Connects to my.example.com on port 22 via SSH to start a new session
distant launch my.example.com

# After the session is established, you can perform different operations
# on the remote machine via `distant action {command} [args]`
distant action copy path/to/file new/path/to/file
distant action run -- echo 'Hello, this is from the other side'
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
