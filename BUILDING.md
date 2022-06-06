# Building from Source

## Dependencies

* `make` - needed to build openssl via vendor feature
* `perl` - needed to build openssl via vendor feature

## Using Cargo

A debug build is straightforward:

```bash
cargo build
```

A release build is also straightforward:

```bash
cargo build --release
```

If you want to install the binary locally from source:

```bash
# Where you are currently an the root of the project
cargo install --path .
```

## Completely static binary

To compile a completely static binary (not linked to libc), we need to target
musl using:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --target x86_64-unknown-linux-musl
```

### Mac-specific

Note that on Mac OS X you will need to install musl-gcc:

```bash
brew install FiloSottile/musl-cross/musl-cross
```

And to do a strip (on Mac), use the musl strip:

```bash
x86_64-linux-musl-gcc target/x86_64-unknown-linux-musl/release/distant
```

At the moment, this is not possible to build on M1 Macs: 
https://github.com/FiloSottile/homebrew-musl-cross/issues/23

## Using Docker

From the root of the repository, run the below, replacing `VERSION` with a
version like `0.16.4`:

```bash
docker build -t chipsenkbeil/distant:VERSION .
```
