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

/// Change to a path on the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Change {
    /// Unix timestamp (in seconds) when the server was notified of this change (not when the
    /// change occurred)
    pub timestamp: u64,

    /// Label describing the kind of change
    pub kind: ChangeKind,

    /// Path that was changed
    pub path: PathBuf,

    /// Additional details associated with the change
    #[serde(default, skip_serializing_if = "ChangeDetails::is_empty")]
    pub details: ChangeDetails,
}

/// Optional details about a change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case", deny_unknown_fields)]
pub struct ChangeDetails {
    /// Clarity on type of attribute change that occurred (for kind == attribute).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute: Option<ChangeDetailsAttribute>,

    /// When event is renaming, this will be populated with the resulting name
    /// when we know both the old and new names (for kind == rename)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed: Option<PathBuf>,

    /// Unix timestamps (in seconds) related to the change. For other platforms, their timestamps
    /// are converted into a Unix timestamp format.
    ///
    /// * For create events, this represents the `ctime` field from stat (or equivalent on other platforms).
    /// * For modify events, this represents the `mtime` field from stat (or equivalent on other platforms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,

    /// Optional information about the change that is typically platform-specific.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
}

impl ChangeDetails {
    /// Returns true if no details are contained within.
    pub fn is_empty(&self) -> bool {
        self.attribute.is_none()
            && self.renamed.is_none()
            && self.timestamp.is_none()
            && self.extra.is_none()
    }
}

/// Specific details about modification
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeDetailsAttribute {
    Ownership,
    Permissions,
    Timestamp,
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

    /// A file, directory, or something else was deleted
    Delete,

    /// A file's content was modified
    Modify,

    /// A file was opened
    Open,

    /// A file, directory, or something else was renamed in some way
    Rename,

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

    /// Returns true if kind is part of the create family.
    pub fn is_create(&self) -> bool {
        matches!(self, Self::Create)
    }

    /// Returns true if kind is part of the delete family.
    pub fn is_delete(&self) -> bool {
        matches!(self, Self::Delete)
    }

    /// Returns true if kind is part of the modify family.
    pub fn is_modify(&self) -> bool {
        matches!(self, Self::Attribute | Self::Modify)
    }

    /// Returns true if kind is part of the rename family.
    pub fn is_rename(&self) -> bool {
        matches!(self, Self::Rename)
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
            ChangeKind::Modify,
            ChangeKind::Open,
            ChangeKind::Rename,
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
    //! Tests for ChangeKind (categorization predicates, variants, BitOr), ChangeKindSet
    //! (set operations, parsing, display, From conversions, equality, hashing),
    //! ChangeDetails (is_empty), ChangeDetailsAttribute (serde), and Change struct (serde).

    use super::*;

    mod change_kind_set {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let set = ChangeKindSet::new([ChangeKind::CloseWrite]);

            let value = serde_json::to_value(set).unwrap();
            assert_eq!(value, serde_json::json!(["close_write"]));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!(["close_write"]);

            let set: ChangeKindSet = serde_json::from_value(value).unwrap();
            assert_eq!(set, ChangeKindSet::new([ChangeKind::CloseWrite]));
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let set = ChangeKindSet::new([ChangeKind::CloseWrite]);

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
                rmp_serde::encode::to_vec_named(&ChangeKindSet::new([ChangeKind::CloseWrite]))
                    .unwrap();

            let set: ChangeKindSet = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(set, ChangeKindSet::new([ChangeKind::CloseWrite]));
        }
    }

    mod change_kind {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let kind = ChangeKind::CloseWrite;

            let value = serde_json::to_value(kind).unwrap();
            assert_eq!(value, serde_json::json!("close_write"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("close_write");

            let kind: ChangeKind = serde_json::from_value(value).unwrap();
            assert_eq!(kind, ChangeKind::CloseWrite);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let kind = ChangeKind::CloseWrite;

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
            let buf = rmp_serde::encode::to_vec_named(&ChangeKind::CloseWrite).unwrap();

            let kind: ChangeKind = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(kind, ChangeKind::CloseWrite);
        }

        #[test]
        fn is_access_should_return_true_for_access_family() {
            assert!(ChangeKind::Access.is_access());
            assert!(ChangeKind::CloseWrite.is_access());
            assert!(ChangeKind::CloseNoWrite.is_access());
            assert!(ChangeKind::Open.is_access());
        }

        #[test]
        fn is_access_should_return_false_for_non_access_kinds() {
            assert!(!ChangeKind::Create.is_access());
            assert!(!ChangeKind::Delete.is_access());
            assert!(!ChangeKind::Modify.is_access());
            assert!(!ChangeKind::Attribute.is_access());
            assert!(!ChangeKind::Rename.is_access());
            assert!(!ChangeKind::Unknown.is_access());
        }

        #[test]
        fn is_create_should_return_true_only_for_create() {
            assert!(ChangeKind::Create.is_create());
            assert!(!ChangeKind::Delete.is_create());
            assert!(!ChangeKind::Modify.is_create());
        }

        #[test]
        fn is_delete_should_return_true_only_for_delete() {
            assert!(ChangeKind::Delete.is_delete());
            assert!(!ChangeKind::Create.is_delete());
            assert!(!ChangeKind::Modify.is_delete());
        }

        #[test]
        fn is_modify_should_return_true_for_modify_family() {
            assert!(ChangeKind::Modify.is_modify());
            assert!(ChangeKind::Attribute.is_modify());
        }

        #[test]
        fn is_modify_should_return_false_for_non_modify_kinds() {
            assert!(!ChangeKind::Create.is_modify());
            assert!(!ChangeKind::Delete.is_modify());
            assert!(!ChangeKind::Access.is_modify());
            assert!(!ChangeKind::Rename.is_modify());
            assert!(!ChangeKind::Unknown.is_modify());
        }

        #[test]
        fn is_rename_should_return_true_only_for_rename() {
            assert!(ChangeKind::Rename.is_rename());
            assert!(!ChangeKind::Create.is_rename());
            assert!(!ChangeKind::Modify.is_rename());
        }

        #[test]
        fn is_unknown_should_return_true_only_for_unknown() {
            assert!(ChangeKind::Unknown.is_unknown());
            assert!(!ChangeKind::Create.is_unknown());
            assert!(!ChangeKind::Access.is_unknown());
        }

        #[test]
        fn variants_should_return_all_variant_names() {
            let names = ChangeKind::variants();
            assert_eq!(names.len(), 10);
            assert!(names.contains(&"access"));
            assert!(names.contains(&"attribute"));
            assert!(names.contains(&"close_write"));
            assert!(names.contains(&"close_no_write"));
            assert!(names.contains(&"create"));
            assert!(names.contains(&"delete"));
            assert!(names.contains(&"modify"));
            assert!(names.contains(&"open"));
            assert!(names.contains(&"rename"));
            assert!(names.contains(&"unknown"));
        }

        #[test]
        fn all_should_return_sorted_vec_of_all_variants() {
            let all = ChangeKind::all();
            assert_eq!(all.len(), 10);
            // Verify it is sorted by checking each consecutive pair
            for window in all.windows(2) {
                assert!(window[0] <= window[1]);
            }
        }

        #[test]
        fn bitor_should_produce_change_kind_set_with_both_kinds() {
            let set = ChangeKind::Access | ChangeKind::Create;
            assert!(set.contains(&ChangeKind::Access));
            assert!(set.contains(&ChangeKind::Create));
            assert_eq!(set.len(), 2);
        }

        #[test]
        fn bitor_same_kind_should_produce_set_with_one_element() {
            let set = ChangeKind::Access | ChangeKind::Access;
            assert!(set.contains(&ChangeKind::Access));
            assert_eq!(set.len(), 1);
        }
    }

    mod change_kind_set_ops {
        use super::*;

        #[test]
        fn bitor_set_with_set_should_merge() {
            let a = ChangeKindSet::new([ChangeKind::Access, ChangeKind::Create]);
            let b = ChangeKindSet::new([ChangeKind::Delete, ChangeKind::Modify]);
            let merged = a | b;
            assert_eq!(merged.len(), 4);
            assert!(merged.contains(&ChangeKind::Access));
            assert!(merged.contains(&ChangeKind::Create));
            assert!(merged.contains(&ChangeKind::Delete));
            assert!(merged.contains(&ChangeKind::Modify));
        }

        #[test]
        fn bitor_set_with_kind_should_add_kind() {
            let set = ChangeKindSet::new([ChangeKind::Access]);
            let result = set | ChangeKind::Delete;
            assert_eq!(result.len(), 2);
            assert!(result.contains(&ChangeKind::Access));
            assert!(result.contains(&ChangeKind::Delete));
        }

        #[test]
        fn bitor_kind_with_set_should_add_kind() {
            let set = ChangeKindSet::new([ChangeKind::Access]);
            let result = ChangeKind::Delete | set;
            assert_eq!(result.len(), 2);
            assert!(result.contains(&ChangeKind::Access));
            assert!(result.contains(&ChangeKind::Delete));
        }

        #[test]
        fn sub_set_from_set_should_remove_elements() {
            let a =
                ChangeKindSet::new([ChangeKind::Access, ChangeKind::Create, ChangeKind::Delete]);
            let b = ChangeKindSet::new([ChangeKind::Create]);
            let result = a - b;
            assert_eq!(result.len(), 2);
            assert!(result.contains(&ChangeKind::Access));
            assert!(result.contains(&ChangeKind::Delete));
            assert!(!result.contains(&ChangeKind::Create));
        }

        #[test]
        fn sub_ref_set_from_ref_set_should_remove_elements() {
            let a = ChangeKindSet::new([ChangeKind::Access, ChangeKind::Create]);
            let b = ChangeKindSet::new([ChangeKind::Access]);
            let result = &a - &b;
            assert_eq!(result.len(), 1);
            assert!(result.contains(&ChangeKind::Create));
        }

        #[test]
        fn from_str_should_parse_single_kind() {
            let set: ChangeKindSet = "access".parse().unwrap();
            assert_eq!(set.len(), 1);
            assert!(set.contains(&ChangeKind::Access));
        }

        #[test]
        fn from_str_should_parse_comma_separated_kinds() {
            let set: ChangeKindSet = "access,create,delete".parse().unwrap();
            assert_eq!(set.len(), 3);
            assert!(set.contains(&ChangeKind::Access));
            assert!(set.contains(&ChangeKind::Create));
            assert!(set.contains(&ChangeKind::Delete));
        }

        #[test]
        fn from_str_should_trim_whitespace() {
            let set: ChangeKindSet = " access , create ".parse().unwrap();
            assert_eq!(set.len(), 2);
            assert!(set.contains(&ChangeKind::Access));
            assert!(set.contains(&ChangeKind::Create));
        }

        #[test]
        fn from_str_should_fail_on_invalid_kind() {
            let result: Result<ChangeKindSet, _> = "not_a_kind".parse();
            assert!(result.is_err());
        }

        #[test]
        fn display_should_produce_sorted_comma_separated_output() {
            let set = ChangeKindSet::new([ChangeKind::Delete, ChangeKind::Access]);
            let s = set.to_string();
            assert_eq!(s, "access,delete");
        }

        #[test]
        fn empty_set_should_display_as_empty_string() {
            let set = ChangeKindSet::empty();
            assert_eq!(set.to_string(), "");
        }

        #[test]
        fn all_should_contain_every_variant() {
            let all = ChangeKindSet::all();
            assert_eq!(all.len(), 10);
            assert!(all.contains(&ChangeKind::Access));
            assert!(all.contains(&ChangeKind::Attribute));
            assert!(all.contains(&ChangeKind::CloseWrite));
            assert!(all.contains(&ChangeKind::CloseNoWrite));
            assert!(all.contains(&ChangeKind::Create));
            assert!(all.contains(&ChangeKind::Delete));
            assert!(all.contains(&ChangeKind::Modify));
            assert!(all.contains(&ChangeKind::Open));
            assert!(all.contains(&ChangeKind::Rename));
            assert!(all.contains(&ChangeKind::Unknown));
        }

        #[test]
        fn default_should_be_empty() {
            let set = ChangeKindSet::default();
            assert!(set.is_empty());
        }

        #[test]
        fn from_single_kind_should_create_set_with_one_element() {
            let set = ChangeKindSet::from(ChangeKind::Create);
            assert_eq!(set.len(), 1);
            assert!(set.contains(&ChangeKind::Create));
        }

        #[test]
        fn from_vec_should_create_set() {
            let set = ChangeKindSet::from(vec![ChangeKind::Access, ChangeKind::Modify]);
            assert_eq!(set.len(), 2);
        }

        #[test]
        fn into_sorted_vec_should_return_sorted_kinds() {
            let set =
                ChangeKindSet::new([ChangeKind::Unknown, ChangeKind::Access, ChangeKind::Create]);
            let sorted = set.into_sorted_vec();
            for window in sorted.windows(2) {
                assert!(window[0] <= window[1]);
            }
            assert_eq!(sorted.len(), 3);
        }

        #[test]
        fn eq_should_compare_by_contents() {
            let a = ChangeKindSet::new([ChangeKind::Access, ChangeKind::Create]);
            let b = ChangeKindSet::new([ChangeKind::Create, ChangeKind::Access]);
            assert_eq!(a, b);
        }

        #[test]
        fn hash_should_be_consistent_for_equal_sets() {
            use std::collections::hash_map::DefaultHasher;
            let a = ChangeKindSet::new([ChangeKind::Access, ChangeKind::Create]);
            let b = ChangeKindSet::new([ChangeKind::Create, ChangeKind::Access]);

            let mut ha = DefaultHasher::new();
            a.hash(&mut ha);
            let mut hb = DefaultHasher::new();
            b.hash(&mut hb);
            assert_eq!(ha.finish(), hb.finish());
        }
    }

    mod change_details {
        use super::*;

        #[test]
        fn is_empty_should_return_true_when_all_fields_are_none() {
            let details = ChangeDetails::default();
            assert!(details.is_empty());
        }

        #[test]
        fn is_empty_should_return_false_when_attribute_is_set() {
            let details = ChangeDetails {
                attribute: Some(ChangeDetailsAttribute::Ownership),
                ..Default::default()
            };
            assert!(!details.is_empty());
        }

        #[test]
        fn is_empty_should_return_false_when_renamed_is_set() {
            let details = ChangeDetails {
                renamed: Some(PathBuf::from("/new/name")),
                ..Default::default()
            };
            assert!(!details.is_empty());
        }

        #[test]
        fn is_empty_should_return_false_when_timestamp_is_set() {
            let details = ChangeDetails {
                timestamp: Some(12345),
                ..Default::default()
            };
            assert!(!details.is_empty());
        }

        #[test]
        fn is_empty_should_return_false_when_extra_is_set() {
            let details = ChangeDetails {
                extra: Some("extra info".to_string()),
                ..Default::default()
            };
            assert!(!details.is_empty());
        }
    }

    mod change_details_attribute {
        use super::*;

        #[test]
        fn should_serialize_all_variants_to_json() {
            assert_eq!(
                serde_json::to_value(ChangeDetailsAttribute::Ownership).unwrap(),
                serde_json::json!("ownership")
            );
            assert_eq!(
                serde_json::to_value(ChangeDetailsAttribute::Permissions).unwrap(),
                serde_json::json!("permissions")
            );
            assert_eq!(
                serde_json::to_value(ChangeDetailsAttribute::Timestamp).unwrap(),
                serde_json::json!("timestamp")
            );
        }

        #[test]
        fn should_deserialize_all_variants_from_json() {
            assert_eq!(
                serde_json::from_value::<ChangeDetailsAttribute>(serde_json::json!("ownership"))
                    .unwrap(),
                ChangeDetailsAttribute::Ownership
            );
            assert_eq!(
                serde_json::from_value::<ChangeDetailsAttribute>(serde_json::json!("permissions"))
                    .unwrap(),
                ChangeDetailsAttribute::Permissions
            );
            assert_eq!(
                serde_json::from_value::<ChangeDetailsAttribute>(serde_json::json!("timestamp"))
                    .unwrap(),
                ChangeDetailsAttribute::Timestamp
            );
        }
    }

    mod change {
        use super::*;

        #[test]
        fn should_serialize_to_json_without_empty_details() {
            let change = Change {
                timestamp: 1000,
                kind: ChangeKind::Create,
                path: PathBuf::from("/tmp/file.txt"),
                details: ChangeDetails::default(),
            };

            let value = serde_json::to_value(&change).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "timestamp": 1000,
                    "kind": "create",
                    "path": "/tmp/file.txt",
                })
            );
            // details should be omitted when empty
            assert!(value.get("details").is_none());
        }

        #[test]
        fn should_serialize_to_json_with_non_empty_details() {
            let change = Change {
                timestamp: 2000,
                kind: ChangeKind::Rename,
                path: PathBuf::from("/old/name"),
                details: ChangeDetails {
                    renamed: Some(PathBuf::from("/new/name")),
                    ..Default::default()
                },
            };

            let value = serde_json::to_value(&change).unwrap();
            assert_eq!(value["details"]["renamed"], serde_json::json!("/new/name"));
        }

        #[test]
        fn should_roundtrip_through_json() {
            let change = Change {
                timestamp: 3000,
                kind: ChangeKind::Modify,
                path: PathBuf::from("/some/path"),
                details: ChangeDetails {
                    attribute: Some(ChangeDetailsAttribute::Permissions),
                    timestamp: Some(4000),
                    extra: Some("info".to_string()),
                    ..Default::default()
                },
            };

            let json = serde_json::to_value(&change).unwrap();
            let restored: Change = serde_json::from_value(json).unwrap();
            assert_eq!(restored, change);
        }

        #[test]
        fn should_deserialize_from_json_without_details_field() {
            let value = serde_json::json!({
                "timestamp": 500,
                "kind": "delete",
                "path": "/gone",
            });

            let change: Change = serde_json::from_value(value).unwrap();
            assert_eq!(change.timestamp, 500);
            assert_eq!(change.kind, ChangeKind::Delete);
            assert_eq!(change.path, PathBuf::from("/gone"));
            assert!(change.details.is_empty());
        }
    }
}
