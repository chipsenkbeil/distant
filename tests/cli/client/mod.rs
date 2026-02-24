//! Module declarations for all CLI client subcommand integration tests.
//!
//! Each submodule tests a specific `distant` CLI subcommand (filesystem operations,
//! process management, connection management, etc.).

mod connect;
mod fs_copy;
mod fs_exists;
mod fs_make_dir;
mod fs_metadata;
mod fs_read_directory;
mod fs_read_file;
mod fs_remove;
mod fs_rename;
mod fs_search;
mod fs_set_permissions;
mod fs_watch;
mod fs_write;
mod kill;
mod launch;
mod select;
mod shell;
mod spawn;
mod status;
mod system_info;
mod version;
