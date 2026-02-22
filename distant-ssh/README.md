# distant ssh

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-ssh.svg
[distant_crates_lnk]: https://crates.io/crates/distant-ssh
[distant_doc_img]: https://docs.rs/distant-ssh/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-ssh

Library provides native ssh integration into the
[`distant`](https://github.com/chipsenkbeil/distant) binary.

## Details

The `distant-ssh` library supplies functionality to speak over the `ssh`
protocol using `distant` and spawn `distant` servers on remote machines using
`ssh`.

## Installation

You can import the dependency by adding the following to your `Cargo.toml`:

```toml
[dependencies]
distant-ssh = "0.20"
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
