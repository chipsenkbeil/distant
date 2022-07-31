use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use strum::AsRefStr;

/// Represents information about a single entry within a directory
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DirEntry {
    /// Represents the full path to the entry
    pub path: PathBuf,

    /// Represents the type of the entry as a file/dir/symlink
    pub file_type: FileType,

    /// Depth at which this entry was created relative to the root (0 being immediately within
    /// root)
    pub depth: usize,
}

/// Represents the type associated with a dir entry
#[derive(Copy, Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum FileType {
    Dir,
    File,
    Symlink,
}
