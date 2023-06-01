# distant local

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.64.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-local.svg
[distant_crates_lnk]: https://crates.io/crates/distant-local
[distant_doc_img]: https://docs.rs/distant-local/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-local
[distant_rustc_img]: https://img.shields.io/badge/distant_local-rustc_1.64+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2022/09/22/Rust-1.64.0.html

## Details

The `distant-local` library acts as the primary implementation of a distant
server that powers the CLI. The logic acts on the local machine of the server
and is designed to be used as the foundation for distant operation handling.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-local = "0.20"
```

## Examples

```rust,no_run
// Create a server API handler to be used with the server
let handler = distant_local::initialize_handler().unwrap();
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
