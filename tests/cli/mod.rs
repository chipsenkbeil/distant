//! CLI integration test module tree.

mod api;
mod client;
mod config;
mod connect;
mod errors;
mod format;
mod generate;
mod global_opts;
mod help;
mod launch;
mod manager;
#[cfg(any(
    feature = "mount-fuse",
    feature = "mount-nfs",
    feature = "mount-windows-cloud-files",
    feature = "mount-macos-file-provider",
))]
mod mount;
mod server;
mod ssh;
mod tunnel;
