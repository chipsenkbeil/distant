use serde::{Deserialize, Serialize};

use crate::protocol::utils;

/// Options for reading a file.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct ReadFileOptions {
    /// Byte offset to start reading from. None means start of file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,

    /// Number of bytes to read. None means read to end of file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len: Option<u64>,
}

/// Options for writing a file.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct WriteFileOptions {
    /// Byte offset to write at. None means write from start (or end if append).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,

    /// If true, append to end of file instead of overwriting.
    #[serde(skip_serializing_if = "utils::is_false")]
    pub append: bool,
}
