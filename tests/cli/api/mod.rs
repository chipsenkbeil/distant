//! Module declarations for all JSON API endpoint integration tests.
//!
//! Each submodule tests a specific API endpoint via JSON requests sent to an `ApiProcess`.

mod copy;
mod dir_create;
mod dir_read;
mod exists;
mod file_append;
mod file_append_text;
mod file_read;
mod file_read_text;
mod file_write;
mod file_write_text;
mod metadata;
mod proc_spawn;
mod remove;
mod rename;
mod search;
mod set_permissions;
mod system_info;
mod version;
mod watch;
