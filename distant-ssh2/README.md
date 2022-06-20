# distant ssh2

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.51.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-ssh2.svg
[distant_crates_lnk]: https://crates.io/crates/distant-ssh2
[distant_doc_img]: https://docs.rs/distant-ssh2/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-ssh2
[distant_rustc_img]: https://img.shields.io/badge/distant_ssh2-rustc_1.51+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2021/03/25/Rust-1.51.0.html

Library provides native ssh integration into the
[`distant`](https://github.com/chipsenkbeil/distant) binary.

🚧 **(Alpha stage software) This library is in rapid development and may break or change frequently!** 🚧

## Details

The `distant-ssh2` library supplies functionality to 

- Asynchronous in nature, powered by [`tokio`](https://tokio.rs/)
- Data is serialized to send across the wire via [`CBOR`](https://cbor.io/)
- Encryption & authentication are handled via
  [XChaCha20Poly1305](https://tools.ietf.org/html/rfc8439) for an authenticated
  encryption scheme via
  [RustCrypto/ChaCha20Poly1305](https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305)

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-ssh2 = "0.17"
```

## Examples

Below is an example of connecting to an ssh server and producing a distant
session that uses ssh without a distant server binary:

```rust
use distant_ssh2::Ssh2Session;

// Using default ssh session arguments to establish a connection
let mut ssh_session = Ssh2Session::connect("example.com", Default::default()).expect("Failed to connect");

// Authenticating with the server is a separate step
// 1. You can pass defaults and authentication and host verification will
//    be done over stderr
// 2. You can provide your own handlers for programmatic engagement
ssh_session.authenticate(Default::default()).await.expect("Failed to authenticate");

// Convert into an ssh client session (no distant server required)
let session = ssh_session.into_ssh_client_session().await.expect("Failed to convert session");
```

Below is an example of connecting to an ssh server and producing a distant
session that spawns a distant server binary and then connects to it:

```rust
use distant_ssh2::Ssh2Session;

// Using default ssh session arguments to establish a connection
let mut ssh_session = Ssh2Session::connect("example.com", Default::default()).expect("Failed to connect");

// Authenticating with the server is a separate step
// 1. You can pass defaults and authentication and host verification will
//    be done over stderr
// 2. You can provide your own handlers for programmatic engagement
ssh_session.authenticate(Default::default()).await.expect("Failed to authenticate");

// Convert into a distant session, which involves spawning a distant server
// using the current ssh connection and then establishing a new connection
// to the distant server
//
// This takes in `IntoDistantSessionOpts` to specify the server's bin path,
// arguments, timeout, and whether or not to spawn using a login shell
let session = ssh_session.into_distant_session(Default::default()).await.expect("Failed to convert session");
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
