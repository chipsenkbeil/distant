# Building from Source

## Using Cargo

```bash
cargo build
```

## Completely static binary

To compile a completely static binary (not linked to libc), we need to target
musl using:

```bash
cargo build --target x86_64-unknown-linux-musl
```

At the moment, this is not possible to build on M1 Macs: 
https://github.com/FiloSottile/homebrew-musl-cross/issues/23
