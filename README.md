# distant

Binary to connect with a remote machine to edit files and run programs.

## Details

The `distant` binary supplies both a server and client component as well as
a command to start a server and configure the local client to be able to
talk to the server.

- Asynchronous in nature, powered by [`tokio`](https://tokio.rs/)
- Data is compressed to send across the wire via [`CBOR`](https://cbor.io/)
- Encryption & authentication are handled via [`orion`](https://crates.io/crates/orion)
    - [XChaCha20Poly1305](https://cryptopp.com/wiki/XChaCha20Poly1305) for an authenticated encryption scheme
    - [BLAKE2b-256](https://www.blake2.net/) in keyed mode for a second authentication
    - [Elliptic Curve Diffie-Hellman](https://en.wikipedia.org/wiki/Elliptic-curve_Diffie%E2%80%93Hellman) (ECDH) for key exchange

## Examples

Launch a remote instance of `distant` by SSHing into another machine and
starting the `distant` executable:

```bash
# Connects to my.example.com on port 22 via SSH to start a new session
distant launch my.example.com

# After the session is established, you can perform different operations
# on the remote machine via `distant send {command} [args]`
distant send copy path/to/file new/path/to/file
distant send proc-run echo 'Hello, this is from the other side'
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
