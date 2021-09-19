# distant core

Library that powers the [`distant`](https://github.com/chipsenkbeil/distant)
binary.

🚧 **(Alpha stage software) This library is in rapid development and may break or change frequently!** 🚧

## Details

The `distant` library supplies a mixture of functionality and data to run
servers that operate on remote machines and clients that talk to them.

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
distant-core = "0.13"
```

## Features

Currently, the library supports the following features:

- `structopt`: generates [`StructOpt`](https://github.com/TeXitoi/structopt)
  bindings for `RequestData` (used by cli to expose request actions)

By default, no features are enabled on the library.

## Examples

Below is an example of connecting to a distant server over TCP:

```rust
use distant_core::{Request, RequestData, Session, SessionInfo};
use std::path::PathBuf;

// Load our session using the environment variables
//
// DISTANT_HOST     = "..."
// DISTANT_PORT     = "..."
// DISTANT_AUTH_KEY = "..."
let mut session = Session::tcp_connect(SessionInfo::from_environment()?).await.unwrap();

// Send a request under a specific name and wait for a response
let tenant = "my name";
let req = Request::new(
  tenant, 
  vec![RequestData::FileReadText { path: PathBuf::from("some/path") }]
);

let res = session.send(req).await.unwrap();
println!("Response: {:?}", res);
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
