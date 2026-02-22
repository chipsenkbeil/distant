# distant host

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-host.svg
[distant_crates_lnk]: https://crates.io/crates/distant-host
[distant_doc_img]: https://docs.rs/distant-host/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-host

## Details

The `distant-host` library acts as the primary implementation of a distant
server that powers the CLI. The logic acts on the local machine of the server
and is designed to be used as the foundation for distant operation handling.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-host = "0.20"
```

## Examples

```rust,no_run
use distant_host::{Config, new_handler};

// Create a server API handler to be used with the server
let handler = new_handler(Config::default()).unwrap();
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
