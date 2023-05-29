use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FromIterator;
use std::ops::{BitOr, Sub};
use std::path::PathBuf;
use std::str::FromStr;

use derive_more::{Deref, DerefMut, IntoIterator};
use serde::{Deserialize, Serialize};
use strum::{EnumString, EnumVariantNames, VariantNames};

/// Change to one or more paths on the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Change {
    /// Label describing the kind of change
    pub kind: ChangeKind,

    /// Paths that were changed
    pub paths: Vec<PathBuf>,
}

/// Represents a label attached to a [`Change`] that describes the kind of change.
///
/// This mirrors events seen from `incron`.
#[derive(
    Copy,
    Clone,
    Debug,
    strum::Display,
    EnumString,
    EnumVariantNames,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum ChangeKind {
    /// A file was read
    Access,

    /// A file's or directory's attributes were changed
    Attribute,

    /// A file open for writing was closed
    CloseWrite,

    /// A file not open for writing was closed
    CloseNoWrite,

    /// A file, directory, or something else was created within a watched directory
    Create,

    /// A file, directory, or something else was deleted within a watched directory
    Delete,

    /// A watched file or directory was deleted
    DeleteSelf,

    /// A file's content was modified
    Modify,

    /// A file, directory, or something else was moved out of a watched directory
    MoveFrom,

    /// A watched file or directory was moved
    MoveSelf,

    /// A file, directory, or something else was moved into a watched directory
    MoveTo,

    /// A file was opened
    Open,

    /// Catch-all for any other change
    Unknown,
}

impl ChangeKind {
    /// Returns a list of all variants as str names
    pub const fn variants() -> &'static [&'static str] {
        Self::VARIANTS
    }

    /// Returns a list of all variants as a vec
    pub fn all() -> Vec<ChangeKind> {
        ChangeKindSet::all().into_sorted_vec()
    }

    /// Returns true if kind is part of the access family.
    pub fn is_access(&self) -> bool {
        matches!(
            self,
            Self::Access | Self::CloseWrite | Self::CloseNoWrite | Self::Open
        )
    }

    /// Returns true if kind is part of the modify family.
    pub fn is_modify(&self) -> bool {
        matches!(self, Self::Attribute | Self::Modify)
    }

    /// Returns true if kind is unknown.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl BitOr for ChangeKind {
    type Output = ChangeKindSet;

    fn bitor(self, rhs: Self) -> Self::Output {
        let mut set = ChangeKindSet::empty();
        set.insert(self);
        set.insert(rhs);
        set
    }
}

/// Represents a distinct set of different change kinds
#[derive(Clone, Debug, Deref, DerefMut, IntoIterator, Serialize, Deserialize)]
pub struct ChangeKindSet(HashSet<ChangeKind>);

impl ChangeKindSet {
    pub fn new(set: impl IntoIterator<Item = ChangeKind>) -> Self {
        set.into_iter().collect()
    }

    /// Produces an empty set of [`ChangeKind`]
    pub fn empty() -> Self {
        Self(HashSet::new())
    }

    /// Produces a set of all [`ChangeKind`]
    pub fn all() -> Self {
        vec![
            ChangeKind::Access,
            ChangeKind::Attribute,
            ChangeKind::CloseWrite,
            ChangeKind::CloseNoWrite,
            ChangeKind::Create,
            ChangeKind::Delete,
            ChangeKind::DeleteSelf,
            ChangeKind::Modify,
            ChangeKind::MoveFrom,
            ChangeKind::MoveSelf,
            ChangeKind::MoveTo,
            ChangeKind::Open,
            ChangeKind::Unknown,
        ]
        .into_iter()
        .collect()
    }

    /// Consumes set and returns a sorted vec of the kinds of changes
    pub fn into_sorted_vec(self) -> Vec<ChangeKind> {
        let mut v = self.0.into_iter().collect::<Vec<_>>();
        v.sort();
        v
    }
}

impl fmt::Display for ChangeKindSet {
    /// Outputs a comma-separated series of [`ChangeKind`] as string that are sorted
    /// such that this will always be consistent output
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut kinds = self
            .0
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>();
        kinds.sort_unstable();
        write!(f, "{}", kinds.join(","))
    }
}

impl PartialEq for ChangeKindSet {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl Eq for ChangeKindSet {}

impl Hash for ChangeKindSet {
    /// Hashes based on the output of [`fmt::Display`]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_string().hash(state);
    }
}

impl BitOr<ChangeKindSet> for ChangeKindSet {
    type Output = Self;

    fn bitor(mut self, rhs: ChangeKindSet) -> Self::Output {
        self.extend(rhs.0);
        self
    }
}

impl BitOr<ChangeKind> for ChangeKindSet {
    type Output = Self;

    fn bitor(mut self, rhs: ChangeKind) -> Self::Output {
        self.0.insert(rhs);
        self
    }
}

impl BitOr<ChangeKindSet> for ChangeKind {
    type Output = ChangeKindSet;

    fn bitor(self, rhs: ChangeKindSet) -> Self::Output {
        rhs | self
    }
}

impl Sub<ChangeKindSet> for ChangeKindSet {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        ChangeKindSet(&self.0 - &other.0)
    }
}

impl Sub<&'_ ChangeKindSet> for &ChangeKindSet {
    type Output = ChangeKindSet;

    fn sub(self, other: &ChangeKindSet) -> Self::Output {
        ChangeKindSet(&self.0 - &other.0)
    }
}

impl FromStr for ChangeKindSet {
    type Err = strum::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut change_set = HashSet::new();

        for word in s.split(',') {
            change_set.insert(ChangeKind::from_str(word.trim())?);
        }

        Ok(ChangeKindSet(change_set))
    }
}

impl FromIterator<ChangeKind> for ChangeKindSet {
    fn from_iter<I: IntoIterator<Item = ChangeKind>>(iter: I) -> Self {
        let mut change_set = HashSet::new();

        for i in iter {
            change_set.insert(i);
        }

        ChangeKindSet(change_set)
    }
}

impl From<ChangeKind> for ChangeKindSet {
    fn from(change_kind: ChangeKind) -> Self {
        let mut set = Self::empty();
        set.insert(change_kind);
        set
    }
}

impl From<Vec<ChangeKind>> for ChangeKindSet {
    fn from(changes: Vec<ChangeKind>) -> Self {
        changes.into_iter().collect()
    }
}

impl Default for ChangeKindSet {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod change_kind_set {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let set = ChangeKindSet::new([ChangeKind::MoveTo]);

            let value = serde_json::to_value(set).unwrap();
            assert_eq!(value, serde_json::json!(["move_to"]));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!(["move_to"]);

            let set: ChangeKindSet = serde_json::from_value(value).unwrap();
            assert_eq!(set, ChangeKindSet::new([ChangeKind::MoveTo]));
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let set = ChangeKindSet::new([ChangeKind::MoveTo]);

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&set).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf =
                rmp_serde::encode::to_vec_named(&ChangeKindSet::new([ChangeKind::MoveTo])).unwrap();

            let set: ChangeKindSet = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(set, ChangeKindSet::new([ChangeKind::MoveTo]));
        }
    }

    mod change_kind {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let kind = ChangeKind::MoveTo;

            let value = serde_json::to_value(kind).unwrap();
            assert_eq!(value, serde_json::json!("move_to"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("move_to");

            let kind: ChangeKind = serde_json::from_value(value).unwrap();
            assert_eq!(kind, ChangeKind::MoveTo);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let kind = ChangeKind::MoveTo;

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&kind).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&ChangeKind::MoveTo).unwrap();

            let kind: ChangeKind = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(kind, ChangeKind::MoveTo);
        }
    }
}
