# distant core

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.68.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-core.svg
[distant_crates_lnk]: https://crates.io/crates/distant-core
[distant_doc_img]: https://docs.rs/distant-core/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-core
[distant_rustc_img]: https://img.shields.io/badge/distant_core-rustc_1.68+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2023/03/09/Rust-1.68.0.html

## Details

The `distant-core` library supplies the client and server interfaces along with
a client implementation for distant. The library exposes an API that downstream
libraries such as `distant-local` and `distant-ssh2` can implement to provide a
distant-compatible interface.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-core = "0.20"
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
