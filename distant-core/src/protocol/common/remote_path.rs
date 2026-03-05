use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A path on a remote machine.
///
/// This is a thin newtype over [`String`] with no encoding assumptions.
/// Each plugin interprets the path according to its own context
/// (e.g. Docker always treats paths as Unix; SSH uses auto-detection).
///
/// `RemotePath` serializes as a plain string, making it wire-compatible
/// with the previous `PathBuf`-based protocol.
///
/// # Examples
///
/// ```
/// use distant_core::protocol::RemotePath;
///
/// let path = RemotePath::new("/home/user/file.txt");
/// assert_eq!(path.as_str(), "/home/user/file.txt");
/// assert_eq!(path.to_string(), "/home/user/file.txt");
///
/// let from_string: RemotePath = String::from("/tmp/data").into();
/// assert_eq!(from_string.as_str(), "/tmp/data");
/// ```
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemotePath(String);

impl RemotePath {
    /// Creates a new remote path from anything that can be converted into a [`String`].
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Returns the path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this value and returns the inner [`String`].
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for RemotePath {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RemotePath {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<PathBuf> for RemotePath {
    fn from(p: PathBuf) -> Self {
        Self(p.to_string_lossy().into_owned())
    }
}

impl From<&Path> for RemotePath {
    fn from(p: &Path) -> Self {
        Self(p.to_string_lossy().into_owned())
    }
}

impl From<RemotePath> for PathBuf {
    fn from(p: RemotePath) -> Self {
        PathBuf::from(p.0)
    }
}

impl AsRef<str> for RemotePath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_should_create_from_str() {
        let path = RemotePath::new("/home/user/file.txt");
        assert_eq!(path.as_str(), "/home/user/file.txt");
    }

    #[test]
    fn new_should_create_from_string() {
        let path = RemotePath::new(String::from("/tmp/data"));
        assert_eq!(path.as_str(), "/tmp/data");
    }

    #[test]
    fn display_should_output_inner_string() {
        let path = RemotePath::new("/some/path");
        assert_eq!(format!("{path}"), "/some/path");
    }

    #[test]
    fn from_string_should_convert() {
        let path: RemotePath = String::from("/a/b").into();
        assert_eq!(path.as_str(), "/a/b");
    }

    #[test]
    fn from_str_ref_should_convert() {
        let path: RemotePath = "/x/y".into();
        assert_eq!(path.as_str(), "/x/y");
    }

    #[test]
    fn from_pathbuf_should_convert() {
        let pb = PathBuf::from("/foo/bar");
        let path: RemotePath = pb.into();
        assert_eq!(path.as_str(), "/foo/bar");
    }

    #[test]
    fn into_pathbuf_should_convert() {
        let path = RemotePath::new("/foo/bar");
        let pb: PathBuf = path.into();
        assert_eq!(pb, PathBuf::from("/foo/bar"));
    }

    #[test]
    fn into_string_should_return_inner() {
        let path = RemotePath::new("/abc");
        assert_eq!(path.into_string(), "/abc");
    }

    #[test]
    fn should_serialize_as_plain_string_json() {
        let path = RemotePath::new("/home/user");
        let value = serde_json::to_value(&path).unwrap();
        assert_eq!(value, serde_json::json!("/home/user"));
    }

    #[test]
    fn should_deserialize_from_plain_string_json() {
        let value = serde_json::json!("/home/user");
        let path: RemotePath = serde_json::from_value(value).unwrap();
        assert_eq!(path.as_str(), "/home/user");
    }

    #[test]
    fn should_roundtrip_through_msgpack() {
        let original = RemotePath::new("/data/file.txt");
        let buf = rmp_serde::encode::to_vec_named(&original).unwrap();
        let restored: RemotePath = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn should_be_wire_compatible_with_pathbuf() {
        // A RemotePath should deserialize from bytes that were serialized as a PathBuf,
        // ensuring backward compatibility with the previous protocol.
        let pb = PathBuf::from("/compat/test");
        let buf = rmp_serde::encode::to_vec_named(&pb).unwrap();
        let path: RemotePath = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(path.as_str(), "/compat/test");
    }
}
