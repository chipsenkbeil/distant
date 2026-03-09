# Building from Source

## Dependencies

* Rust 1.88.0+ (install via [rustup](https://rustup.rs/))

## Using Cargo

```bash
# Debug build
cargo build

# Release build (optimized for size: opt-level=z, LTO, strip)
cargo build --release

# Build all workspace members with all features
cargo build --all-features --workspace

# Build a specific crate
cargo build -p distant-core
cargo build -p distant-docker
cargo build -p distant-host
cargo build -p distant-ssh

# Install the binary locally from source
cargo install --path .
```

## Using Nix

If you have [Nix](https://nixos.org/) with flakes enabled, you can build
without manually installing Rust or any native dependencies:

```bash
# Build the release binary
nix build

# The binary will be at ./result/bin/distant

# Enter a development shell with all tools available
nix develop
```

## Completely static binary

To compile a completely static binary (not linked to libc), target musl:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

### Mac-specific

On macOS you will need to install musl-gcc:

```bash
brew install FiloSottile/musl-cross/musl-cross
```

And to strip (on Mac), use the musl strip:

```bash
x86_64-linux-musl-strip target/x86_64-unknown-linux-musl/release/distant
```

## Cross-compilation

The CI release workflow builds for the following targets:

| Platform | Target | Notes |
|----------|--------|-------|
| macOS | `x86_64-apple-darwin` | |
| macOS | `aarch64-apple-darwin` | |
| macOS | Universal binary | `lipo` merge of both architectures |
| Windows | `x86_64-pc-windows-msvc` | |
| Windows | `aarch64-pc-windows-msvc` | |
| Linux | `x86_64-unknown-linux-gnu` | |
| Linux | `aarch64-unknown-linux-gnu` | Needs `gcc-aarch64-linux-gnu` |
| Linux | `armv7-unknown-linux-gnueabihf` | Needs `gcc-arm-linux-gnueabihf` |
| Linux (musl) | `x86_64-unknown-linux-musl` | Needs `musl-tools` |
| Linux (musl) | `aarch64-unknown-linux-musl` | Uses `cross` |
| FreeBSD | `x86_64-unknown-freebsd` | Uses `cross` |
