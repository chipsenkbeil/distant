use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FromIterator;
use std::ops::{BitOr, Sub};
use std::path::PathBuf;
use std::str::FromStr;

use derive_more::{Deref, DerefMut, IntoIterator};
use notify::event::Event as NotifyEvent;
use notify::EventKind as NotifyEventKind;
use serde::{Deserialize, Serialize};
use strum::{EnumString, EnumVariantNames, VariantNames};

/// Change to one or more paths on the filesystem
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Change {
    /// Label describing the kind of change
    pub kind: ChangeKind,

    /// Paths that were changed
    pub paths: Vec<PathBuf>,
}

#[cfg(feature = "schemars")]
impl Change {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Change)
    }
}

impl From<NotifyEvent> for Change {
    fn from(x: NotifyEvent) -> Self {
        Self {
            kind: x.kind.into(),
            paths: x.paths,
        }
    }
}

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
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum ChangeKind {
    /// Something about a file or directory was accessed, but
    /// no specific details were known
    Access,

    /// A file was closed for executing
    AccessCloseExecute,

    /// A file was closed for reading
    AccessCloseRead,

    /// A file was closed for writing
    AccessCloseWrite,

    /// A file was opened for executing
    AccessOpenExecute,

    /// A file was opened for reading
    AccessOpenRead,

    /// A file was opened for writing
    AccessOpenWrite,

    /// A file or directory was read
    AccessRead,

    /// The access time of a file or directory was changed
    AccessTime,

    /// A file, directory, or something else was created
    Create,

    /// The content of a file or directory changed
    Content,

    /// The data of a file or directory was modified, but
    /// no specific details were known
    Data,

    /// The metadata of a file or directory was modified, but
    /// no specific details were known
    Metadata,

    /// Something about a file or directory was modified, but
    /// no specific details were known
    Modify,

    /// A file, directory, or something else was removed
    Remove,

    /// A file or directory was renamed, but no specific details were known
    Rename,

    /// A file or directory was renamed, and the provided paths
    /// are the source and target in that order (from, to)
    RenameBoth,

    /// A file or directory was renamed, and the provided path
    /// is the origin of the rename (before being renamed)
    RenameFrom,

    /// A file or directory was renamed, and the provided path
    /// is the result of the rename
    RenameTo,

    /// A file's size changed
    Size,

    /// The ownership of a file or directory was changed
    Ownership,

    /// The permissions of a file or directory was changed
    Permissions,

    /// The write or modify time of a file or directory was changed
    WriteTime,

    // Catchall in case we have no insight as to the type of change
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

    /// Returns true if the change is a kind of access
    pub fn is_access_kind(&self) -> bool {
        self.is_open_access_kind()
            || self.is_close_access_kind()
            || matches!(self, Self::Access | Self::AccessRead)
    }

    /// Returns true if the change is a kind of open access
    pub fn is_open_access_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessOpenExecute | Self::AccessOpenRead | Self::AccessOpenWrite
        )
    }

    /// Returns true if the change is a kind of close access
    pub fn is_close_access_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessCloseExecute | Self::AccessCloseRead | Self::AccessCloseWrite
        )
    }

    /// Returns true if the change is a kind of creation
    pub fn is_create_kind(&self) -> bool {
        matches!(self, Self::Create)
    }

    /// Returns true if the change is a kind of modification
    pub fn is_modify_kind(&self) -> bool {
        self.is_data_modify_kind() || self.is_metadata_modify_kind() || matches!(self, Self::Modify)
    }

    /// Returns true if the change is a kind of data modification
    pub fn is_data_modify_kind(&self) -> bool {
        matches!(self, Self::Content | Self::Data | Self::Size)
    }

    /// Returns true if the change is a kind of metadata modification
    pub fn is_metadata_modify_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessTime
                | Self::Metadata
                | Self::Ownership
                | Self::Permissions
                | Self::WriteTime
        )
    }

    /// Returns true if the change is a kind of rename
    pub fn is_rename_kind(&self) -> bool {
        matches!(
            self,
            Self::Rename | Self::RenameBoth | Self::RenameFrom | Self::RenameTo
        )
    }

    /// Returns true if the change is a kind of removal
    pub fn is_remove_kind(&self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Returns true if the change kind is unknown
    pub fn is_unknown_kind(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

#[cfg(feature = "schemars")]
impl ChangeKind {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ChangeKind)
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

impl From<NotifyEventKind> for ChangeKind {
    fn from(x: NotifyEventKind) -> Self {
        use notify::event::{
            AccessKind, AccessMode, DataChange, MetadataKind, ModifyKind, RenameMode,
        };
        match x {
            // File/directory access events
            NotifyEventKind::Access(AccessKind::Read) => Self::AccessRead,
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Execute)) => {
                Self::AccessOpenExecute
            }
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Read)) => Self::AccessOpenRead,
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Write)) => Self::AccessOpenWrite,
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Execute)) => {
                Self::AccessCloseExecute
            }
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Read)) => Self::AccessCloseRead,
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Write)) => Self::AccessCloseWrite,
            NotifyEventKind::Access(_) => Self::Access,

            // File/directory creation events
            NotifyEventKind::Create(_) => Self::Create,

            // Rename-oriented events
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::Both)) => Self::RenameBoth,
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::From)) => Self::RenameFrom,
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::To)) => Self::RenameTo,
            NotifyEventKind::Modify(ModifyKind::Name(_)) => Self::Rename,

            // Data-modification events
            NotifyEventKind::Modify(ModifyKind::Data(DataChange::Content)) => Self::Content,
            NotifyEventKind::Modify(ModifyKind::Data(DataChange::Size)) => Self::Size,
            NotifyEventKind::Modify(ModifyKind::Data(_)) => Self::Data,

            // Metadata-modification events
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => {
                Self::AccessTime
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)) => {
                Self::WriteTime
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::Permissions)) => {
                Self::Permissions
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::Ownership)) => {
                Self::Ownership
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(_)) => Self::Metadata,

            // General modification events
            NotifyEventKind::Modify(_) => Self::Modify,

            // File/directory removal events
            NotifyEventKind::Remove(_) => Self::Remove,

            // Catch-all for other events
            NotifyEventKind::Any | NotifyEventKind::Other => Self::Unknown,
        }
    }
}

/// Represents a distinct set of different change kinds
#[derive(Clone, Debug, Deref, DerefMut, IntoIterator, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ChangeKindSet(HashSet<ChangeKind>);

impl ChangeKindSet {
    /// Produces an empty set of [`ChangeKind`]
    pub fn empty() -> Self {
        Self(HashSet::new())
    }

    /// Produces a set of all [`ChangeKind`]
    pub fn all() -> Self {
        vec![
            ChangeKind::Access,
            ChangeKind::AccessCloseExecute,
            ChangeKind::AccessCloseRead,
            ChangeKind::AccessCloseWrite,
            ChangeKind::AccessOpenExecute,
            ChangeKind::AccessOpenRead,
            ChangeKind::AccessOpenWrite,
            ChangeKind::AccessRead,
            ChangeKind::AccessTime,
            ChangeKind::Create,
            ChangeKind::Content,
            ChangeKind::Data,
            ChangeKind::Metadata,
            ChangeKind::Modify,
            ChangeKind::Remove,
            ChangeKind::Rename,
            ChangeKind::RenameBoth,
            ChangeKind::RenameFrom,
            ChangeKind::RenameTo,
            ChangeKind::Size,
            ChangeKind::Ownership,
            ChangeKind::Permissions,
            ChangeKind::WriteTime,
            ChangeKind::Unknown,
        ]
        .into_iter()
        .collect()
    }

    /// Produces a changeset containing all of the access kinds
    pub fn access_set() -> Self {
        Self::access_open_set()
            | Self::access_close_set()
            | ChangeKind::AccessRead
            | ChangeKind::Access
    }

    /// Produces a changeset containing all of the open access kinds
    pub fn access_open_set() -> Self {
        ChangeKind::AccessOpenExecute | ChangeKind::AccessOpenRead | ChangeKind::AccessOpenWrite
    }

    /// Produces a changeset containing all of the close access kinds
    pub fn access_close_set() -> Self {
        ChangeKind::AccessCloseExecute | ChangeKind::AccessCloseRead | ChangeKind::AccessCloseWrite
    }

    // Produces a changeset containing all of the modification kinds
    pub fn modify_set() -> Self {
        Self::modify_data_set() | Self::modify_metadata_set() | ChangeKind::Modify
    }

    /// Produces a changeset containing all of the data modification kinds
    pub fn modify_data_set() -> Self {
        ChangeKind::Content | ChangeKind::Data | ChangeKind::Size
    }

    /// Produces a changeset containing all of the metadata modification kinds
    pub fn modify_metadata_set() -> Self {
        ChangeKind::AccessTime
            | ChangeKind::Metadata
            | ChangeKind::Ownership
            | ChangeKind::Permissions
            | ChangeKind::WriteTime
    }

    /// Produces a changeset containing all of the rename kinds
    pub fn rename_set() -> Self {
        ChangeKind::Rename | ChangeKind::RenameBoth | ChangeKind::RenameFrom | ChangeKind::RenameTo
    }

    /// Consumes set and returns a sorted vec of the kinds of changes
    pub fn into_sorted_vec(self) -> Vec<ChangeKind> {
        let mut v = self.0.into_iter().collect::<Vec<_>>();
        v.sort();
        v
    }
}

#[cfg(feature = "schemars")]
impl ChangeKindSet {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ChangeKindSet)
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
