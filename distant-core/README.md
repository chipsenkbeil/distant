# distant core

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.61.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-core.svg
[distant_crates_lnk]: https://crates.io/crates/distant-core
[distant_doc_img]: https://docs.rs/distant-core/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-core
[distant_rustc_img]: https://img.shields.io/badge/distant_core-rustc_1.61+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2022/05/19/Rust-1.61.0.html

Library that powers the [`distant`](https://github.com/chipsenkbeil/distant)
binary.

🚧 **(Alpha stage software) This library is in rapid development and may break or change frequently!** 🚧

## Details

The `distant-core` library supplies the client, manager, and server
implementations for use with the distant API in order to communicate with
remote machines and perform actions. This library acts as the primary
implementation that powers the CLI, but is also available for other extensions
like `distant-ssh2`.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-core = "0.19"
```

## Features

Currently, the library supports the following features:

- `clap`: generates [`Clap`](https://github.com/clap-rs) bindings for
  `DistantRequestData` (used by cli to expose request actions)
- `schemars`: derives the `schemars::JsonSchema` interface on
  `DistantMsg`, `DistantRequestData`, and `DistantResponseData` data types

By default, no features are enabled on the library.

## Examples

Below is an example of connecting to a distant server over TCP without any
encryption or authentication:

```rust
use distant_core::{
  DistantClient,
  DistantChannelExt,
  net::{PlainCodec, TcpClientExt},
};
use std::{net::SocketAddr, path::Path};

// Connect to a server located at example.com on port 8080 that is using
// no encryption or authentication (PlainCodec)
let addr: SocketAddr = "example.com:8080".parse().unwrap();
let mut client = DistantClient::connect(addr, PlainCodec).await
  .expect("Failed to connect");

// Append text to a file
// NOTE: This method comes from DistantChannelExt
client.append_file_text(Path::new("path/to/file.txt"), "new contents").await
  .expect("Failed to append to file");
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
