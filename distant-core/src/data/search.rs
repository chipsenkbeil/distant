use super::FileType;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, path::PathBuf, str::FromStr};

/// Id associated with a search
pub type SearchId = u32;

/// Represents a query to perform against the filesystem
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQuery {
    /// Path in which to perform the query
    pub path: PathBuf,

    /// Kind of data to example using conditions
    pub target: SearchQueryTarget,

    /// Condition to meet to be considered a match
    pub condition: SearchQueryCondition,

    /// Options to apply to the query
    #[serde(default)]
    pub options: Vec<SearchQueryOption>,
}

#[cfg(feature = "schemars")]
impl SearchQuery {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQuery)
    }
}

impl FromStr for SearchQuery {
    type Err = serde_json::error::Error;

    /// Parses search query from a JSON string
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

/// Kind of data to examine using conditions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchQueryTarget {
    /// Checks path of file, directory, or symlink
    Path,

    /// Checks contents of files
    Contents,
}

#[cfg(feature = "schemars")]
impl SearchQueryTarget {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryTarget)
    }
}

/// Condition used to find a match in a search query
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum SearchQueryCondition {
    /// Begins with some text
    EndsWith { value: String },

    /// Matches some text exactly
    Equals { value: String },

    /// Matches some regex
    Regex { value: String },

    /// Begins with some text
    StartsWith { value: String },
}

#[cfg(feature = "schemars")]
impl SearchQueryCondition {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryCondition)
    }
}

/// Option associated with a search query
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum SearchQueryOption {
    /// Restrict search to only this file type (more than one can be included)
    FileType { kind: FileType },

    /// Search should follow symbolic links
    FollowSymbolicLinks,

    /// Maximum results to return before stopping the query
    Limit { limit: u64 },

    /// Amount of results to batch before sending back excluding final submission that will always
    /// include the remaining results even if less than pagination request
    Pagination { count: u64 },
}

#[cfg(feature = "schemars")]
impl SearchQueryOption {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryOption)
    }
}

/// Represents a match for a search query
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum SearchQueryMatch {
    /// Matches part of a file's path
    Path(SearchQueryPathMatch),

    /// Matches part of a file's contents
    Contents(SearchQueryContentsMatch),
}

#[cfg(feature = "schemars")]
impl SearchQueryMatch {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryMatch)
    }
}

/// Represents details for a match on a path
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQueryPathMatch {
    /// Path associated with the match
    pub path: PathBuf,

    /// Collection of matches tied to `path` where each submatch's byte offset is relative to
    /// `path`
    pub submatches: Vec<SearchQuerySubmatch>,
}

#[cfg(feature = "schemars")]
impl SearchQueryPathMatch {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryPathMatch)
    }
}

/// Represents details for a match on a file's contents
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQueryContentsMatch {
    /// Path to file whose contents match
    pub path: PathBuf,

    /// Line(s) that matched
    pub lines: SearchQueryMatchData,

    /// Line number where match starts (base index 1)
    pub line_number: u64,

    /// Absolute byte offset corresponding to the start of `lines` in the data being searched
    pub absolute_offset: u64,

    /// Collection of matches tied to `lines` where each submatch's byte offset is relative to
    /// `lines` and not the overall content
    pub submatches: Vec<SearchQuerySubmatch>,
}

#[cfg(feature = "schemars")]
impl SearchQueryContentsMatch {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryContentsMatch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQuerySubmatch {
    /// Content matched by query
    pub r#match: SearchQueryMatchData,

    /// Byte offset representing start of submatch (inclusive)
    pub start: u64,

    /// Byte offset representing end of submatch (exclusive)
    pub end: u64,
}

#[cfg(feature = "schemars")]
impl SearchQuerySubmatch {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQuerySubmatch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchQueryMatchData {
    /// Match represented as UTF-8 text
    Text(String),

    /// Match represented as bytes
    Bytes(Vec<u8>),
}

impl SearchQueryMatchData {
    /// Returns the UTF-8 str reference to the data, if is valid UTF-8
    pub fn to_str(&self) -> Option<&str> {
        match self {
            Self::Text(x) => Some(x),
            Self::Bytes(x) => std::str::from_utf8(x).ok(),
        }
    }

    /// Converts data to a UTF-8 string, replacing any invalid UTF-8 sequences with
    /// [`U+FFFD REPLACEMENT CHARACTER`](https://doc.rust-lang.org/nightly/core/char/const.REPLACEMENT_CHARACTER.html)
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        match self {
            Self::Text(x) => Cow::Borrowed(x),
            Self::Bytes(x) => String::from_utf8_lossy(x),
        }
    }
}

#[cfg(feature = "schemars")]
impl SearchQueryMatchData {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryMatchData)
    }
}
