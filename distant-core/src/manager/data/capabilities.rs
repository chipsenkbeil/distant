use super::ManagerCapabilityKind;
use derive_more::{From, Into, IntoIterator};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashSet,
    hash::{Hash, Hasher},
    ops::{BitAnd, BitOr, BitXor},
    str::FromStr,
};
use strum::{EnumMessage, IntoEnumIterator};

/// Set of supported capabilities for a manager
#[derive(Clone, Debug, From, Into, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct ManagerCapabilities(#[into_iterator(owned, ref)] HashSet<ManagerCapability>);

impl ManagerCapabilities {
    /// Return set of capabilities encompassing all possible capabilities
    pub fn all() -> Self {
        Self(
            ManagerCapabilityKind::iter()
                .map(ManagerCapability::from)
                .collect(),
        )
    }

    /// Return empty set of capabilities
    pub fn none() -> Self {
        Self(HashSet::new())
    }

    /// Returns true if the capability with described kind is included
    pub fn contains(&self, kind: impl AsRef<str>) -> bool {
        let cap = ManagerCapability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.contains(&cap)
    }

    /// Adds the specified capability to the set of capabilities
    ///
    /// * If the set did not have this capability, returns `true`
    /// * If the set did have this capability, returns `false`
    pub fn insert(&mut self, cap: impl Into<ManagerCapability>) -> bool {
        self.0.insert(cap.into())
    }

    /// Removes the capability with the described kind, returning the capability
    pub fn take(&mut self, kind: impl AsRef<str>) -> Option<ManagerCapability> {
        let cap = ManagerCapability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.take(&cap)
    }

    /// Removes the capability with the described kind, returning true if it existed
    pub fn remove(&mut self, kind: impl AsRef<str>) -> bool {
        let cap = ManagerCapability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.remove(&cap)
    }

    /// Converts into vec of capabilities sorted by kind
    pub fn into_sorted_vec(self) -> Vec<ManagerCapability> {
        let mut this = self.0.into_iter().collect::<Vec<_>>();

        this.sort_unstable();

        this
    }
}

#[cfg(feature = "schemars")]
impl ManagerCapabilities {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ManagerCapabilities)
    }
}

impl BitAnd for &ManagerCapabilities {
    type Output = ManagerCapabilities;

    fn bitand(self, rhs: Self) -> Self::Output {
        ManagerCapabilities(self.0.bitand(&rhs.0))
    }
}

impl BitOr for &ManagerCapabilities {
    type Output = ManagerCapabilities;

    fn bitor(self, rhs: Self) -> Self::Output {
        ManagerCapabilities(self.0.bitor(&rhs.0))
    }
}

impl BitOr<ManagerCapability> for &ManagerCapabilities {
    type Output = ManagerCapabilities;

    fn bitor(self, rhs: ManagerCapability) -> Self::Output {
        let mut other = ManagerCapabilities::none();
        other.0.insert(rhs);

        self.bitor(&other)
    }
}

impl BitXor for &ManagerCapabilities {
    type Output = ManagerCapabilities;

    fn bitxor(self, rhs: Self) -> Self::Output {
        ManagerCapabilities(self.0.bitxor(&rhs.0))
    }
}

impl FromIterator<ManagerCapability> for ManagerCapabilities {
    fn from_iter<I: IntoIterator<Item = ManagerCapability>>(iter: I) -> Self {
        let mut this = ManagerCapabilities::none();

        for capability in iter {
            this.0.insert(capability);
        }

        this
    }
}

/// ManagerCapability tied to a manager. A capability is equivalent based on its kind and not
/// description.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ManagerCapability {
    /// Label describing the kind of capability
    pub kind: String,

    /// Information about the capability
    pub description: String,
}

impl ManagerCapability {
    /// Will convert the [`ManagerCapability`]'s `kind` into a known [`ManagerCapabilityKind`] if
    /// possible, returning None if the capability is unknown
    pub fn to_capability_kind(&self) -> Option<ManagerCapabilityKind> {
        ManagerCapabilityKind::from_str(&self.kind).ok()
    }

    /// Returns true if the described capability is unknown
    pub fn is_unknown(&self) -> bool {
        self.to_capability_kind().is_none()
    }
}

impl PartialEq for ManagerCapability {
    fn eq(&self, other: &Self) -> bool {
        self.kind.eq_ignore_ascii_case(&other.kind)
    }
}

impl Eq for ManagerCapability {}

impl PartialOrd for ManagerCapability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ManagerCapability {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .to_ascii_lowercase()
            .cmp(&other.kind.to_ascii_lowercase())
    }
}

impl Hash for ManagerCapability {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.to_ascii_lowercase().hash(state);
    }
}

impl From<ManagerCapabilityKind> for ManagerCapability {
    /// Creates a new capability using the kind's default message
    fn from(kind: ManagerCapabilityKind) -> Self {
        Self {
            kind: kind.to_string(),
            description: kind
                .get_message()
                .map(ToString::to_string)
                .unwrap_or_default(),
        }
    }
}

#[cfg(feature = "schemars")]
impl ManagerCapability {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ManagerCapability)
    }
}

#[cfg(feature = "schemars")]
impl ManagerCapabilityKind {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ManagerCapabilityKind)
    }
}
