<h1 align="center">
  <img src="https://distant.dev/assets/images/distant-with-logo-300x87.png" alt="Distant">

  <a href="https://distant.dev/">Documentation</a> |
  <a href="https://github.com/chipsenkbeil/distant/discussions">Discussion</a>
</h1>

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![CI][distant_ci_img]][distant_ci_lnk] [![RustC 1.70+][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant.svg
[distant_crates_lnk]: https://crates.io/crates/distant
[distant_doc_img]: https://docs.rs/distant/badge.svg
[distant_doc_lnk]: https://docs.rs/distant
[distant_ci_img]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml/badge.svg
[distant_ci_lnk]: https://github.com/chipsenkbeil/distant/actions/workflows/ci.yml
[distant_rustc_img]: https://img.shields.io/badge/distant-rustc_1.70+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2023/06/01/Rust-1.70.0.html

🚧 **(Alpha stage software) This program is in rapid development and may break or change frequently!** 🚧

## Installation

### Unix

```sh
# Need to include -L to follow redirects as this returns 301
curl -L https://sh.distant.dev | sh

# Can also use wget to the same result
wget https://sh.distant.dev | sh
```

See https://distant.dev/getting-started/installation/unix/ for more details.

### Windows

```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser # Optional: Needed to run a remote script the first time
irm sh.distant.dev | iex
```

See https://distant.dev/getting-started/installation/windows/ for more details.

## Usage

```sh
# Start a manager in the background
distant manager listen &

# SSH into a server, start distant, and connect to the distant server
distant launch ssh://example.com

# Read the current working directory
distant fs read .

# Start a shell on the remote machine
distant shell
```

See https://distant.dev/getting-started/usage/ for more details.

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
