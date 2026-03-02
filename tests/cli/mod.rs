//! CLI integration test module tree.
//!
//! Declares all submodules: `api` (JSON protocol tests), `client` (CLI subcommand tests),
//! `generate`, `help`, `manager`, and `server`.

mod api;
mod client;
mod config;
#[cfg(feature = "docker")]
mod docker;
mod errors;
mod format;
mod generate;
mod help;
mod manager;
mod server;
