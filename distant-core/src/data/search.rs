use super::FileType;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashSet, path::PathBuf, str::FromStr};

/// Id associated with a search
pub type SearchId = u32;

/// Represents a query to perform against the filesystem
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQuery {
    /// Kind of data to examine using condition
    #[cfg_attr(feature = "clap", clap(long, value_enum, default_value_t = SearchQueryTarget::Contents))]
    pub target: SearchQueryTarget,

    /// Condition to meet to be considered a match
    #[cfg_attr(feature = "clap", clap(name = "pattern"))]
    pub condition: SearchQueryCondition,

    /// Paths in which to perform the query
    #[cfg_attr(feature = "clap", clap(default_value = "."))]
    pub paths: Vec<PathBuf>,

    /// Options to apply to the query
    #[serde(default)]
    #[cfg_attr(feature = "clap", clap(flatten))]
    pub options: SearchQueryOptions,
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ArgEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "clap", clap(rename_all = "snake_case"))]
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
    /// Text is found anywhere (all regex patterns are escaped)
    Contains { value: String },

    /// Begins with some text (all regex patterns are escaped)
    EndsWith { value: String },

    /// Matches some text exactly (all regex patterns are escaped)
    Equals { value: String },

    /// Any of the conditions match
    Or { value: Vec<SearchQueryCondition> },

    /// Matches some regex
    Regex { value: String },

    /// Begins with some text (all regex patterns are escaped)
    StartsWith { value: String },
}

impl SearchQueryCondition {
    /// Creates a new instance with `Contains` variant
    pub fn contains(value: impl Into<String>) -> Self {
        Self::Contains {
            value: value.into(),
        }
    }

    /// Creates a new instance with `EndsWith` variant
    pub fn ends_with(value: impl Into<String>) -> Self {
        Self::EndsWith {
            value: value.into(),
        }
    }

    /// Creates a new instance with `Equals` variant
    pub fn equals(value: impl Into<String>) -> Self {
        Self::Equals {
            value: value.into(),
        }
    }

    /// Creates a new instance with `Or` variant
    pub fn or<I, C>(value: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<SearchQueryCondition>,
    {
        Self::Or {
            value: value.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Creates a new instance with `Regex` variant
    pub fn regex(value: impl Into<String>) -> Self {
        Self::Regex {
            value: value.into(),
        }
    }

    /// Creates a new instance with `StartsWith` variant
    pub fn starts_with(value: impl Into<String>) -> Self {
        Self::StartsWith {
            value: value.into(),
        }
    }

    /// Converts the condition in a regex string
    pub fn to_regex_string(&self) -> String {
        match self {
            Self::Contains { value } => regex::escape(value),
            Self::EndsWith { value } => format!(r"{}$", regex::escape(value)),
            Self::Equals { value } => format!(r"^{}$", regex::escape(value)),
            Self::Regex { value } => value.to_string(),
            Self::StartsWith { value } => format!(r"^{}", regex::escape(value)),
            Self::Or { value } => {
                let mut s = String::new();
                for (i, condition) in value.iter().enumerate() {
                    if i > 0 {
                        s.push('|');
                    }
                    s.push_str(&condition.to_regex_string());
                }
                s
            }
        }
    }
}

#[cfg(feature = "schemars")]
impl SearchQueryCondition {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryCondition)
    }
}

impl FromStr for SearchQueryCondition {
    type Err = std::convert::Infallible;

    /// Parses search query from a JSON string
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::regex(s))
    }
}

/// Options associated with a search query
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchQueryOptions {
    /// Restrict search to only these file types (otherwise all are allowed)
    #[cfg_attr(feature = "clap", clap(skip))]
    #[serde(default)]
    pub allowed_file_types: HashSet<FileType>,

    /// Regex to use to filter paths being searched to only those that match the include condition
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub include: Option<SearchQueryCondition>,

    /// Regex to use to filter paths being searched to only those that do not match the exclude
    /// condition
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub exclude: Option<SearchQueryCondition>,

    /// Search should follow symbolic links
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub follow_symbolic_links: bool,

    /// Maximum results to return before stopping the query
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub limit: Option<u64>,

    /// Maximum depth (directories) to search
    ///
    /// The smallest depth is 0 and always corresponds to the path given to the new function on
    /// this type. Its direct descendents have depth 1, and their descendents have depth 2, and so
    /// on.
    ///
    /// Note that this will not simply filter the entries of the iterator, but it will actually
    /// avoid descending into directories when the depth is exceeded.
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub max_depth: Option<u64>,

    /// Amount of results to batch before sending back excluding final submission that will always
    /// include the remaining results even if less than pagination request
    #[cfg_attr(feature = "clap", clap(long))]
    #[serde(default)]
    pub pagination: Option<u64>,
}

#[cfg(feature = "schemars")]
impl SearchQueryOptions {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SearchQueryOptions)
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

impl SearchQueryMatch {
    pub fn into_path_match(self) -> Option<SearchQueryPathMatch> {
        match self {
            Self::Path(x) => Some(x),
            _ => None,
        }
    }

    pub fn into_contents_match(self) -> Option<SearchQueryContentsMatch> {
        match self {
            Self::Contents(x) => Some(x),
            _ => None,
        }
    }
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
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "type",
    content = "value"
)]
pub enum SearchQueryMatchData {
    /// Match represented as UTF-8 text
    Text(String),

    /// Match represented as bytes
    Bytes(Vec<u8>),
}

impl SearchQueryMatchData {
    /// Creates a new instance with `Text` variant
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    /// Creates a new instance with `Bytes` variant
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    mod search_query_condition {
        use super::*;
        use test_log::test;

        #[test]
        fn to_regex_string_should_convert_to_appropriate_regex_and_escape_as_needed() {
            assert_eq!(
                SearchQueryCondition::contains("t^es$t").to_regex_string(),
                r"t\^es\$t"
            );
            assert_eq!(
                SearchQueryCondition::ends_with("t^es$t").to_regex_string(),
                r"t\^es\$t$"
            );
            assert_eq!(
                SearchQueryCondition::equals("t^es$t").to_regex_string(),
                r"^t\^es\$t$"
            );
            assert_eq!(
                SearchQueryCondition::or([
                    SearchQueryCondition::contains("t^es$t"),
                    SearchQueryCondition::equals("t^es$t"),
                    SearchQueryCondition::regex("^test$"),
                ])
                .to_regex_string(),
                r"t\^es\$t|^t\^es\$t$|^test$"
            );
            assert_eq!(
                SearchQueryCondition::regex("test").to_regex_string(),
                "test"
            );
            assert_eq!(
                SearchQueryCondition::starts_with("t^es$t").to_regex_string(),
                r"^t\^es\$t"
            );
        }
    }
}
