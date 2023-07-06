# distant ssh2

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.70.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-ssh2.svg
[distant_crates_lnk]: https://crates.io/crates/distant-ssh2
[distant_doc_img]: https://docs.rs/distant-ssh2/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-ssh2
[distant_rustc_img]: https://img.shields.io/badge/distant_ssh2-rustc_1.70+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2023/06/01/Rust-1.70.0.html

Library provides native ssh integration into the
[`distant`](https://github.com/chipsenkbeil/distant) binary.

🚧 **(Alpha stage software) This library is in rapid development and may break or change frequently!** 🚧

## Details

The `distant-ssh2` library supplies functionality to speak over the `ssh`
protocol using `distant` and spawn `distant` servers on remote machines using
`ssh`.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-ssh2 = "0.20"
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
