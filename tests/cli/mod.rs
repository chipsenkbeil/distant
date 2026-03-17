//! CLI integration test module tree.
//!
//! Declares all submodules: `api` (JSON protocol tests), `client` (CLI subcommand tests),
//! `generate`, `help`, `manager`, `pty`, `server`, and `tunnel`.

mod api;
mod client;
mod config;
#[cfg(feature = "docker")]
mod docker;
mod errors;
mod format;
mod generate;
mod global_opts;
mod help;
mod manager;
mod parity;
mod pty;
mod server;
mod service;
mod ssh;
mod tcp_echo_server;
mod tunnel;
