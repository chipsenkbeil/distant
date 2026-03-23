//! macOS FileProvider mount backend (placeholder).
//!
//! The FileProvider framework on macOS 12+ provides deep Finder integration
//! with cloud-style placeholder files, on-demand hydration, and native conflict
//! resolution. However, it **requires** a `.appex` extension inside a `.app`
//! bundle — standalone CLI binaries cannot register as FileProviders.
//!
//! This module is a placeholder for the future implementation. The architecture
//! will consist of three components:
//!
//! 1. **`distant` CLI** — unchanged, launches the app bundle
//! 2. **`DistantMount.app`** — headless app (`LSBackgroundOnly=true`) containing
//!    the appex; also hosts the future menu bar UI
//! 3. **`DistantFileProvider.appex`** — implements `NSFileProviderReplicatedExtension`,
//!    communicates with `distant` via IPC (App Group shared container)
//!
//! ## Bundle structure
//!
//! ```text
//! DistantMount.app/
//!   Contents/
//!     Info.plist                          # LSBackgroundOnly=true
//!     MacOS/distant-mount-host            # Container app binary
//!     PlugIns/
//!       DistantFileProvider.appex/
//!         Contents/
//!           Info.plist                     # NSExtension with fileprovider-nonui
//!           MacOS/distant-file-provider    # Extension binary (Rust)
//! ```
//!
//! ## Implementation plan
//!
//! The extension binary will use `objc2` and `objc2-file-provider` to implement:
//!
//! - `NSFileProviderReplicatedExtension` protocol
//! - Enumerator for paginated directory listings
//! - `fetchContents` / `createItem` / `modifyItem` / `deleteItem`
//! - IPC via App Group shared container for connection credentials
//!
//! Build infrastructure will use a shell script (not Xcode):
//! 1. `cargo build` produces `distant-mount-host` and `distant-file-provider`
//! 2. Post-build assembles the bundle directory structure
//! 3. Code sign with `codesign` (ad-hoc for dev, proper for distribution)
//!
//! ## Dependencies (when implemented)
//!
//! ```toml
//! objc2 = { version = "0.6", optional = true }
//! objc2-file-provider = { version = "0.3", optional = true }
//! objc2-foundation = { version = "0.3", optional = true }
//! block2 = { version = "0.6", optional = true }
//! ```
