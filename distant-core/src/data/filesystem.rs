use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use std::{fs::FileType as StdFileType, path::PathBuf};
use strum::AsRefStr;

/// Represents information about a single entry within a directory
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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

#[cfg(feature = "schemars")]
impl DirEntry {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(DirEntry)
    }
}

/// Represents the type associated with a dir entry
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum FileType {
    Dir,
    File,
    Symlink,
}

impl From<StdFileType> for FileType {
    fn from(ft: StdFileType) -> Self {
        if ft.is_dir() {
            Self::Dir
        } else if ft.is_symlink() {
            Self::Symlink
        } else {
            Self::File
        }
    }
}

#[cfg(feature = "schemars")]
impl FileType {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(FileType)
    }
}
