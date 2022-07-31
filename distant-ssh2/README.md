# distant ssh2

[![Crates.io][distant_crates_img]][distant_crates_lnk] [![Docs.rs][distant_doc_img]][distant_doc_lnk] [![Rustc 1.61.0][distant_rustc_img]][distant_rustc_lnk]

[distant_crates_img]: https://img.shields.io/crates/v/distant-ssh2.svg
[distant_crates_lnk]: https://crates.io/crates/distant-ssh2
[distant_doc_img]: https://docs.rs/distant-ssh2/badge.svg
[distant_doc_lnk]: https://docs.rs/distant-ssh2
[distant_rustc_img]: https://img.shields.io/badge/distant_ssh2-rustc_1.61+-lightgray.svg
[distant_rustc_lnk]: https://blog.rust-lang.org/2022/05/19/Rust-1.61.0.html

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
distant-ssh2 = "0.17"
```

## Examples

Below is an example of connecting to an ssh server and translating between ssh
protocol and distant protocol:

```rust
use distant_ssh2::{LocalSshAuthHandler, Ssh, SshOpts};

// Using default ssh arguments to establish a connection
let mut ssh = Ssh::connect("example.com", SshOpts::default())
  .expect("Failed to connect");

// Authenticating with the server is a separate step
// 1. You can pass the local handler and authentication and host verification
//    will be done over stderr
// 2. You can provide your own handlers for programmatic engagement
ssh.authenticate(LocalSshAuthHandler).await
  .expect("Failed to authenticate");

// Convert into an ssh client session (no distant server required)
let client = ssh.into_distant_client().await
  .expect("Failed to convert into distant client");
```

Below is an example of connecting to an ssh server, spawning a distant server
on the remote machine, and connecting to the distant server:

```rust
use distant_ssh2::{DistantLaunchOpts, LocalSshAuthHandler, Ssh, SshOpts};

// Using default ssh arguments to establish a connection
let mut ssh = Ssh::connect("example.com", SshOpts::default())
  .expect("Failed to connect");

// Authenticating with the server is a separate step
// 1. You can pass the local handler and authentication and host verification
//    will be done over stderr
// 2. You can provide your own handlers for programmatic engagement
ssh.authenticate(LocalSshAuthHandler).await
  .expect("Failed to authenticate");

// Convert into a distant session, which involves spawning a distant server
// using the current ssh connection and then establishing a new connection
// to the distant server
//
// This takes in `DistantLaunchOpts` to specify the server's bin path,
// arguments, timeout, and whether or not to spawn using a login shell
let client = ssh.launch_and_connect(DistantLaunchOpts::default()).await
  .expect("Failed to spawn server or connect to it");
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
