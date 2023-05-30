use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::common::FileType;

/// Id associated with a search
pub type SearchId = u32;

/// Represents a query to perform against the filesystem
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Kind of data to examine using condition
    pub target: SearchQueryTarget,

    /// Condition to meet to be considered a match
    pub condition: SearchQueryCondition,

    /// Paths in which to perform the query
    pub paths: Vec<PathBuf>,

    /// Options to apply to the query
    #[serde(default)]
    pub options: SearchQueryOptions,
}

/// Kind of data to examine using conditions
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchQueryTarget {
    /// Checks path of file, directory, or symlink
    Path,

    /// Checks contents of files
    Contents,
}

/// Condition used to find a match in a search query
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl FromStr for SearchQueryCondition {
    type Err = std::convert::Infallible;

    /// Parses search query from a JSON string
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::regex(s))
    }
}

/// Options associated with a search query
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQueryOptions {
    /// Restrict search to only these file types (otherwise all are allowed).
    pub allowed_file_types: HashSet<FileType>,

    /// Regex to use to filter paths being searched to only those that match the include condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<SearchQueryCondition>,

    /// Regex to use to filter paths being searched to only those that do not match the exclude.
    /// condition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<SearchQueryCondition>,

    /// If true, will search upward through parent directories rather than the traditional downward
    /// search that recurses through all children directories.
    ///
    /// Note that this will use maximum depth to apply to the reverse direction, and will only look
    /// through each ancestor directory's immediate entries. In other words, this will not result
    /// in recursing through sibling directories.
    ///
    /// An upward search will ALWAYS search the contents of a directory, so this means providing a
    /// path to a directory will search its entries EVEN if the max_depth is 0.
    pub upward: bool,

    /// Search should follow symbolic links.
    pub follow_symbolic_links: bool,

    /// Maximum results to return before stopping the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,

    /// Maximum depth (directories) to search
    ///
    /// The smallest depth is 0 and always corresponds to the path given to the new function on
    /// this type. Its direct descendents have depth 1, and their descendents have depth 2, and so
    /// on.
    ///
    /// Note that this will not simply filter the entries of the iterator, but it will actually
    /// avoid descending into directories when the depth is exceeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u64>,

    /// Amount of results to batch before sending back excluding final submission that will always
    /// include the remaining results even if less than pagination request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<u64>,
}

/// Represents a match for a search query
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Represents details for a match on a path
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQueryPathMatch {
    /// Path associated with the match
    pub path: PathBuf,

    /// Collection of matches tied to `path` where each submatch's byte offset is relative to
    /// `path`
    pub submatches: Vec<SearchQuerySubmatch>,
}

/// Represents details for a match on a file's contents
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuerySubmatch {
    /// Content matched by query
    pub r#match: SearchQueryMatchData,

    /// Byte offset representing start of submatch (inclusive)
    pub start: u64,

    /// Byte offset representing end of submatch (exclusive)
    pub end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQueryMatchData {
    /// Match represented as bytes
    Bytes(Vec<u8>),

    /// Match represented as UTF-8 text
    Text(String),
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

#[cfg(test)]
mod tests {
    use super::*;

    mod search_query {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let query = SearchQuery {
                target: SearchQueryTarget::Contents,
                condition: SearchQueryCondition::equals("hello world"),
                paths: vec![PathBuf::from("path1"), PathBuf::from("path2")],
                options: SearchQueryOptions::default(),
            };

            let value = serde_json::to_value(query).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "target": "contents",
                    "condition": {
                        "type": "equals",
                        "value": "hello world",
                    },
                    "paths": ["path1", "path2"],
                    "options": {
                        "allowed_file_types": [],
                        "upward": false,
                        "follow_symbolic_links": false,
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "target": "contents",
                "condition": {
                    "type": "equals",
                    "value": "hello world",
                },
                "paths": ["path1", "path2"],
                "options": {
                    "allowed_file_types": [],
                    "upward": false,
                    "follow_symbolic_links": false,
                },
            });

            let query: SearchQuery = serde_json::from_value(value).unwrap();
            assert_eq!(
                query,
                SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path1"), PathBuf::from("path2")],
                    options: SearchQueryOptions::default(),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let query = SearchQuery {
                target: SearchQueryTarget::Contents,
                condition: SearchQueryCondition::equals("hello world"),
                paths: vec![PathBuf::from("path1"), PathBuf::from("path2")],
                options: SearchQueryOptions::default(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&query).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQuery {
                target: SearchQueryTarget::Contents,
                condition: SearchQueryCondition::equals("hello world"),
                paths: vec![PathBuf::from("path1"), PathBuf::from("path2")],
                options: SearchQueryOptions::default(),
            })
            .unwrap();

            let query: SearchQuery = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                query,
                SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path1"), PathBuf::from("path2")],
                    options: SearchQueryOptions::default(),
                }
            );
        }
    }

    mod search_query_target {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let target = SearchQueryTarget::Contents;
            let value = serde_json::to_value(target).unwrap();
            assert_eq!(value, serde_json::json!("contents"));

            let target = SearchQueryTarget::Path;
            let value = serde_json::to_value(target).unwrap();
            assert_eq!(value, serde_json::json!("path"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("contents");
            let target: SearchQueryTarget = serde_json::from_value(value).unwrap();
            assert_eq!(target, SearchQueryTarget::Contents);

            let value = serde_json::json!("path");
            let target: SearchQueryTarget = serde_json::from_value(value).unwrap();
            assert_eq!(target, SearchQueryTarget::Path);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let target = SearchQueryTarget::Contents;
            let _ = rmp_serde::encode::to_vec_named(&target).unwrap();

            let target = SearchQueryTarget::Path;
            let _ = rmp_serde::encode::to_vec_named(&target).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryTarget::Contents).unwrap();
            let target: SearchQueryTarget = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(target, SearchQueryTarget::Contents);

            let buf = rmp_serde::encode::to_vec_named(&SearchQueryTarget::Path).unwrap();
            let target: SearchQueryTarget = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(target, SearchQueryTarget::Path);
        }
    }

    mod search_query_condition {
        use super::*;

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

        mod contains {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::contains("some text");

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "contains",
                        "value": "some text",
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "contains",
                    "value": "some text",
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(condition, SearchQueryCondition::contains("some text"));
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::contains("some text");

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf =
                    rmp_serde::encode::to_vec_named(&SearchQueryCondition::contains("some text"))
                        .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(condition, SearchQueryCondition::contains("some text"));
            }
        }

        mod ends_with {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::ends_with("some text");

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "ends_with",
                        "value": "some text",
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "ends_with",
                    "value": "some text",
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(condition, SearchQueryCondition::ends_with("some text"));
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::ends_with("some text");

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf =
                    rmp_serde::encode::to_vec_named(&SearchQueryCondition::ends_with("some text"))
                        .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(condition, SearchQueryCondition::ends_with("some text"));
            }
        }

        mod equals {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::equals("some text");

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "equals",
                        "value": "some text",
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "equals",
                    "value": "some text",
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(condition, SearchQueryCondition::equals("some text"));
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::equals("some text");

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf =
                    rmp_serde::encode::to_vec_named(&SearchQueryCondition::equals("some text"))
                        .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(condition, SearchQueryCondition::equals("some text"));
            }
        }

        mod or {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::or([
                    SearchQueryCondition::starts_with("start text"),
                    SearchQueryCondition::ends_with("end text"),
                ]);

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "or",
                        "value": [
                            { "type": "starts_with", "value": "start text" },
                            { "type": "ends_with", "value": "end text" },
                        ],
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "or",
                    "value": [
                        { "type": "starts_with", "value": "start text" },
                        { "type": "ends_with", "value": "end text" },
                    ],
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(
                    condition,
                    SearchQueryCondition::or([
                        SearchQueryCondition::starts_with("start text"),
                        SearchQueryCondition::ends_with("end text"),
                    ])
                );
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::or([
                    SearchQueryCondition::starts_with("start text"),
                    SearchQueryCondition::ends_with("end text"),
                ]);

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf = rmp_serde::encode::to_vec_named(&SearchQueryCondition::or([
                    SearchQueryCondition::starts_with("start text"),
                    SearchQueryCondition::ends_with("end text"),
                ]))
                .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(
                    condition,
                    SearchQueryCondition::or([
                        SearchQueryCondition::starts_with("start text"),
                        SearchQueryCondition::ends_with("end text"),
                    ])
                );
            }
        }

        mod regex {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::regex("some text");

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "regex",
                        "value": "some text",
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "regex",
                    "value": "some text",
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(condition, SearchQueryCondition::regex("some text"));
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::regex("some text");

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf =
                    rmp_serde::encode::to_vec_named(&SearchQueryCondition::regex("some text"))
                        .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(condition, SearchQueryCondition::regex("some text"));
            }
        }

        mod starts_with {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let condition = SearchQueryCondition::starts_with("some text");

                let value = serde_json::to_value(condition).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "starts_with",
                        "value": "some text",
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "starts_with",
                    "value": "some text",
                });

                let condition: SearchQueryCondition = serde_json::from_value(value).unwrap();
                assert_eq!(condition, SearchQueryCondition::starts_with("some text"));
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let condition = SearchQueryCondition::starts_with("some text");

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&condition).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf = rmp_serde::encode::to_vec_named(&SearchQueryCondition::starts_with(
                    "some text",
                ))
                .unwrap();

                let condition: SearchQueryCondition = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(condition, SearchQueryCondition::starts_with("some text"));
            }
        }
    }

    mod search_query_options {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_options_to_json() {
            let options = SearchQueryOptions {
                allowed_file_types: [].into_iter().collect(),
                include: None,
                exclude: None,
                upward: false,
                follow_symbolic_links: false,
                limit: None,
                max_depth: None,
                pagination: None,
            };

            let value = serde_json::to_value(options).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "allowed_file_types": [],
                    "upward": false,
                    "follow_symbolic_links": false,
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_options_to_json() {
            let options = SearchQueryOptions {
                allowed_file_types: [FileType::File].into_iter().collect(),
                include: Some(SearchQueryCondition::Equals {
                    value: String::from("hello"),
                }),
                exclude: Some(SearchQueryCondition::Contains {
                    value: String::from("world"),
                }),
                upward: true,
                follow_symbolic_links: true,
                limit: Some(u64::MAX),
                max_depth: Some(u64::MAX),
                pagination: Some(u64::MAX),
            };

            let value = serde_json::to_value(options).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "allowed_file_types": ["file"],
                    "include": {
                        "type": "equals",
                        "value": "hello",
                    },
                    "exclude": {
                        "type": "contains",
                        "value": "world",
                    },
                    "upward": true,
                    "follow_symbolic_links": true,
                    "limit": u64::MAX,
                    "max_depth": u64::MAX,
                    "pagination": u64::MAX,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_options_from_json() {
            let value = serde_json::json!({
                "allowed_file_types": [],
                "upward": false,
                "follow_symbolic_links": false,
            });

            let options: SearchQueryOptions = serde_json::from_value(value).unwrap();
            assert_eq!(
                options,
                SearchQueryOptions {
                    allowed_file_types: [].into_iter().collect(),
                    include: None,
                    exclude: None,
                    upward: false,
                    follow_symbolic_links: false,
                    limit: None,
                    max_depth: None,
                    pagination: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_options_from_json() {
            let value = serde_json::json!({
                "allowed_file_types": ["file"],
                "include": {
                    "type": "equals",
                    "value": "hello",
                },
                "exclude": {
                    "type": "contains",
                    "value": "world",
                },
                "upward": true,
                "follow_symbolic_links": true,
                "limit": u64::MAX,
                "max_depth": u64::MAX,
                "pagination": u64::MAX,
            });

            let options: SearchQueryOptions = serde_json::from_value(value).unwrap();
            assert_eq!(
                options,
                SearchQueryOptions {
                    allowed_file_types: [FileType::File].into_iter().collect(),
                    include: Some(SearchQueryCondition::Equals {
                        value: String::from("hello"),
                    }),
                    exclude: Some(SearchQueryCondition::Contains {
                        value: String::from("world"),
                    }),
                    upward: true,
                    follow_symbolic_links: true,
                    limit: Some(u64::MAX),
                    max_depth: Some(u64::MAX),
                    pagination: Some(u64::MAX),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_options_to_msgpack() {
            let options = SearchQueryOptions {
                allowed_file_types: [].into_iter().collect(),
                include: None,
                exclude: None,
                upward: false,
                follow_symbolic_links: false,
                limit: None,
                max_depth: None,
                pagination: None,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&options).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_options_to_msgpack() {
            let options = SearchQueryOptions {
                allowed_file_types: [FileType::File].into_iter().collect(),
                include: Some(SearchQueryCondition::Equals {
                    value: String::from("hello"),
                }),
                exclude: Some(SearchQueryCondition::Contains {
                    value: String::from("world"),
                }),
                upward: true,
                follow_symbolic_links: true,
                limit: Some(u64::MAX),
                max_depth: Some(u64::MAX),
                pagination: Some(u64::MAX),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&options).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_options_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryOptions {
                allowed_file_types: [].into_iter().collect(),
                include: None,
                exclude: None,
                upward: false,
                follow_symbolic_links: false,
                limit: None,
                max_depth: None,
                pagination: None,
            })
            .unwrap();

            let options: SearchQueryOptions = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                options,
                SearchQueryOptions {
                    allowed_file_types: [].into_iter().collect(),
                    include: None,
                    exclude: None,
                    upward: false,
                    follow_symbolic_links: false,
                    limit: None,
                    max_depth: None,
                    pagination: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_options_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryOptions {
                allowed_file_types: [FileType::File].into_iter().collect(),
                include: Some(SearchQueryCondition::Equals {
                    value: String::from("hello"),
                }),
                exclude: Some(SearchQueryCondition::Contains {
                    value: String::from("world"),
                }),
                upward: true,
                follow_symbolic_links: true,
                limit: Some(u64::MAX),
                max_depth: Some(u64::MAX),
                pagination: Some(u64::MAX),
            })
            .unwrap();

            let options: SearchQueryOptions = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                options,
                SearchQueryOptions {
                    allowed_file_types: [FileType::File].into_iter().collect(),
                    include: Some(SearchQueryCondition::Equals {
                        value: String::from("hello"),
                    }),
                    exclude: Some(SearchQueryCondition::Contains {
                        value: String::from("world"),
                    }),
                    upward: true,
                    follow_symbolic_links: true,
                    limit: Some(u64::MAX),
                    max_depth: Some(u64::MAX),
                    pagination: Some(u64::MAX),
                }
            );
        }
    }

    mod search_query_match {
        use super::*;

        mod for_path {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let r#match = SearchQueryMatch::Path(SearchQueryPathMatch {
                    path: PathBuf::from("path"),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                });

                let value = serde_json::to_value(r#match).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "path",
                        "path": "path",
                        "submatches": [{
                            "match": "text",
                            "start": 8,
                            "end": 13,
                        }],
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "path",
                    "path": "path",
                    "submatches": [{
                        "match": "text",
                        "start": 8,
                        "end": 13,
                    }],
                });

                let r#match: SearchQueryMatch = serde_json::from_value(value).unwrap();
                assert_eq!(
                    r#match,
                    SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("path"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    })
                );
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let r#match = SearchQueryMatch::Path(SearchQueryPathMatch {
                    path: PathBuf::from("path"),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                });

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&r#match).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf = rmp_serde::encode::to_vec_named(&SearchQueryMatch::Path(
                    SearchQueryPathMatch {
                        path: PathBuf::from("path"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    },
                ))
                .unwrap();

                let r#match: SearchQueryMatch = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(
                    r#match,
                    SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("path"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    })
                );
            }
        }

        mod for_contents {
            use super::*;

            #[test]
            fn should_be_able_to_serialize_to_json() {
                let r#match = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: SearchQueryMatchData::Text(String::from("some text")),
                    line_number: 12,
                    absolute_offset: 24,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                });

                let value = serde_json::to_value(r#match).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({
                        "type": "contents",
                        "path": "path",
                        "lines": "some text",
                        "line_number": 12,
                        "absolute_offset": 24,
                        "submatches": [{
                            "match": "text",
                            "start": 8,
                            "end": 13,
                        }],
                    })
                );
            }

            #[test]
            fn should_be_able_to_deserialize_from_json() {
                let value = serde_json::json!({
                    "type": "contents",
                    "path": "path",
                    "lines": "some text",
                    "line_number": 12,
                    "absolute_offset": 24,
                    "submatches": [{
                        "match": "text",
                        "start": 8,
                        "end": 13,
                    }],
                });

                let r#match: SearchQueryMatch = serde_json::from_value(value).unwrap();
                assert_eq!(
                    r#match,
                    SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path: PathBuf::from("path"),
                        lines: SearchQueryMatchData::Text(String::from("some text")),
                        line_number: 12,
                        absolute_offset: 24,
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    })
                );
            }

            #[test]
            fn should_be_able_to_serialize_to_msgpack() {
                let r#match = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: SearchQueryMatchData::Text(String::from("some text")),
                    line_number: 12,
                    absolute_offset: 24,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                });

                // NOTE: We don't actually check the output here because it's an implementation detail
                // and could change as we change how serialization is done. This is merely to verify
                // that we can serialize since there are times when serde fails to serialize at
                // runtime.
                let _ = rmp_serde::encode::to_vec_named(&r#match).unwrap();
            }

            #[test]
            fn should_be_able_to_deserialize_from_msgpack() {
                // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
                // verify that we are not corrupting or causing issues when serializing on a
                // client/server and then trying to deserialize on the other side. This has happened
                // enough times with minor changes that we need tests to verify.
                let buf = rmp_serde::encode::to_vec_named(&SearchQueryMatch::Contents(
                    SearchQueryContentsMatch {
                        path: PathBuf::from("path"),
                        lines: SearchQueryMatchData::Text(String::from("some text")),
                        line_number: 12,
                        absolute_offset: 24,
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    },
                ))
                .unwrap();

                let r#match: SearchQueryMatch = rmp_serde::decode::from_slice(&buf).unwrap();
                assert_eq!(
                    r#match,
                    SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path: PathBuf::from("path"),
                        lines: SearchQueryMatchData::Text(String::from("some text")),
                        line_number: 12,
                        absolute_offset: 24,
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text(String::from("text")),
                            start: 8,
                            end: 13,
                        }],
                    })
                );
            }
        }
    }

    mod search_query_path_match {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let r#match = SearchQueryPathMatch {
                path: PathBuf::from("path"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            };

            let value = serde_json::to_value(r#match).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "path": "path",
                    "submatches": [{
                        "match": "text",
                        "start": 8,
                        "end": 13,
                    }],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "path": "path",
                "submatches": [{
                    "match": "text",
                    "start": 8,
                    "end": 13,
                }],
            });

            let r#match: SearchQueryPathMatch = serde_json::from_value(value).unwrap();
            assert_eq!(
                r#match,
                SearchQueryPathMatch {
                    path: PathBuf::from("path"),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let r#match = SearchQueryPathMatch {
                path: PathBuf::from("path"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&r#match).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryPathMatch {
                path: PathBuf::from("path"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            })
            .unwrap();

            let r#match: SearchQueryPathMatch = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                r#match,
                SearchQueryPathMatch {
                    path: PathBuf::from("path"),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                }
            );
        }
    }

    mod search_query_contents_match {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let r#match = SearchQueryContentsMatch {
                path: PathBuf::from("path"),
                lines: SearchQueryMatchData::Text(String::from("some text")),
                line_number: 12,
                absolute_offset: 24,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            };

            let value = serde_json::to_value(r#match).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "path": "path",
                    "lines": "some text",
                    "line_number": 12,
                    "absolute_offset": 24,
                    "submatches": [{
                        "match": "text",
                        "start": 8,
                        "end": 13,
                    }],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "path": "path",
                "lines": "some text",
                "line_number": 12,
                "absolute_offset": 24,
                "submatches": [{
                    "match": "text",
                    "start": 8,
                    "end": 13,
                }],
            });

            let r#match: SearchQueryContentsMatch = serde_json::from_value(value).unwrap();
            assert_eq!(
                r#match,
                SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: SearchQueryMatchData::Text(String::from("some text")),
                    line_number: 12,
                    absolute_offset: 24,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let r#match = SearchQueryContentsMatch {
                path: PathBuf::from("path"),
                lines: SearchQueryMatchData::Text(String::from("some text")),
                line_number: 12,
                absolute_offset: 24,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&r#match).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryContentsMatch {
                path: PathBuf::from("path"),
                lines: SearchQueryMatchData::Text(String::from("some text")),
                line_number: 12,
                absolute_offset: 24,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("text")),
                    start: 8,
                    end: 13,
                }],
            })
            .unwrap();

            let r#match: SearchQueryContentsMatch = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                r#match,
                SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: SearchQueryMatchData::Text(String::from("some text")),
                    line_number: 12,
                    absolute_offset: 24,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(String::from("text")),
                        start: 8,
                        end: 13,
                    }],
                }
            );
        }
    }

    mod search_query_submatch {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let data = SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(String::from("some text")),
                start: 12,
                end: 24,
            };

            let value = serde_json::to_value(data).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "match": "some text",
                    "start": 12,
                    "end": 24,
                })
            );

            let data = SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Bytes(vec![1, 2, 3]),
                start: 12,
                end: 24,
            };

            // Do the same for bytes
            let value = serde_json::to_value(data).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "match": [1, 2, 3],
                    "start": 12,
                    "end": 24,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "match": "some text",
                "start": 12,
                "end": 24,
            });

            let submatch: SearchQuerySubmatch = serde_json::from_value(value).unwrap();
            assert_eq!(
                submatch,
                SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("some text")),
                    start: 12,
                    end: 24,
                }
            );

            // Do the same for bytes
            let value = serde_json::json!({
                "match": [1, 2, 3],
                "start": 12,
                "end": 24,
            });

            let submatch: SearchQuerySubmatch = serde_json::from_value(value).unwrap();
            assert_eq!(
                submatch,
                SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Bytes(vec![1, 2, 3]),
                    start: 12,
                    end: 24,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let submatch = SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(String::from("some text")),
                start: 12,
                end: 24,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&submatch).unwrap();

            // Do the same for bytes
            let submatch = SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Bytes(vec![1, 2, 3]),
                start: 12,
                end: 24,
            };

            let _ = rmp_serde::encode::to_vec_named(&submatch).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(String::from("some text")),
                start: 12,
                end: 24,
            })
            .unwrap();

            let submatch: SearchQuerySubmatch = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                submatch,
                SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(String::from("some text")),
                    start: 12,
                    end: 24,
                }
            );

            // Do the same for bytes
            let buf = rmp_serde::encode::to_vec_named(&SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Bytes(vec![1, 2, 3]),
                start: 12,
                end: 24,
            })
            .unwrap();

            let submatch: SearchQuerySubmatch = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                submatch,
                SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Bytes(vec![1, 2, 3]),
                    start: 12,
                    end: 24,
                }
            );
        }
    }

    mod search_query_match_data {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let data = SearchQueryMatchData::Text(String::from("some text"));

            let value = serde_json::to_value(data).unwrap();
            assert_eq!(value, serde_json::json!("some text"));

            // Do the same for bytes
            let data = SearchQueryMatchData::Bytes(vec![1, 2, 3]);

            let value = serde_json::to_value(data).unwrap();
            assert_eq!(value, serde_json::json!([1, 2, 3]));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("some text");

            let data: SearchQueryMatchData = serde_json::from_value(value).unwrap();
            assert_eq!(data, SearchQueryMatchData::Text(String::from("some text")));

            // Do the same for bytes
            let value = serde_json::json!([1, 2, 3]);

            let data: SearchQueryMatchData = serde_json::from_value(value).unwrap();
            assert_eq!(data, SearchQueryMatchData::Bytes(vec![1, 2, 3]));
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let data = SearchQueryMatchData::Text(String::from("some text"));

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&data).unwrap();

            // Do the same for bytes
            let data = SearchQueryMatchData::Bytes(vec![1, 2, 3]);
            let _ = rmp_serde::encode::to_vec_named(&data).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryMatchData::Text(String::from(
                "some text",
            )))
            .unwrap();

            let data: SearchQueryMatchData = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(data, SearchQueryMatchData::Text(String::from("some text")));

            // Do the same for bytes
            let buf = rmp_serde::encode::to_vec_named(&SearchQueryMatchData::Bytes(vec![1, 2, 3]))
                .unwrap();

            let data: SearchQueryMatchData = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(data, SearchQueryMatchData::Bytes(vec![1, 2, 3]));
        }
    }
}
