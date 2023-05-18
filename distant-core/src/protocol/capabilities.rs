use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::{BitAnd, BitOr, BitXor};
use std::str::FromStr;

use derive_more::{From, Into, IntoIterator};
use serde::{Deserialize, Serialize};
use strum::{EnumMessage, IntoEnumIterator};

use super::CapabilityKind;

/// Set of supported capabilities for a server
#[derive(Clone, Debug, From, Into, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Capabilities(#[into_iterator(owned, ref)] HashSet<Capability>);

impl Capabilities {
    /// Return set of capabilities encompassing all possible capabilities
    pub fn all() -> Self {
        Self(CapabilityKind::iter().map(Capability::from).collect())
    }

    /// Return empty set of capabilities
    pub fn none() -> Self {
        Self(HashSet::new())
    }

    /// Returns true if the capability with described kind is included
    pub fn contains(&self, kind: impl AsRef<str>) -> bool {
        let cap = Capability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.contains(&cap)
    }

    /// Adds the specified capability to the set of capabilities
    ///
    /// * If the set did not have this capability, returns `true`
    /// * If the set did have this capability, returns `false`
    pub fn insert(&mut self, cap: impl Into<Capability>) -> bool {
        self.0.insert(cap.into())
    }

    /// Removes the capability with the described kind, returning the capability
    pub fn take(&mut self, kind: impl AsRef<str>) -> Option<Capability> {
        let cap = Capability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.take(&cap)
    }

    /// Removes the capability with the described kind, returning true if it existed
    pub fn remove(&mut self, kind: impl AsRef<str>) -> bool {
        let cap = Capability {
            kind: kind.as_ref().to_string(),
            description: String::new(),
        };
        self.0.remove(&cap)
    }

    /// Converts into vec of capabilities sorted by kind
    pub fn into_sorted_vec(self) -> Vec<Capability> {
        let mut this = self.0.into_iter().collect::<Vec<_>>();

        this.sort_unstable();

        this
    }
}

#[cfg(feature = "schemars")]
impl Capabilities {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Capabilities)
    }
}

impl BitAnd for &Capabilities {
    type Output = Capabilities;

    fn bitand(self, rhs: Self) -> Self::Output {
        Capabilities(self.0.bitand(&rhs.0))
    }
}

impl BitOr for &Capabilities {
    type Output = Capabilities;

    fn bitor(self, rhs: Self) -> Self::Output {
        Capabilities(self.0.bitor(&rhs.0))
    }
}

impl BitOr<Capability> for &Capabilities {
    type Output = Capabilities;

    fn bitor(self, rhs: Capability) -> Self::Output {
        let mut other = Capabilities::none();
        other.0.insert(rhs);

        self.bitor(&other)
    }
}

impl BitXor for &Capabilities {
    type Output = Capabilities;

    fn bitxor(self, rhs: Self) -> Self::Output {
        Capabilities(self.0.bitxor(&rhs.0))
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        let mut this = Capabilities::none();

        for capability in iter {
            this.0.insert(capability);
        }

        this
    }
}

/// Capability tied to a server. A capability is equivalent based on its kind and not description.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Capability {
    /// Label describing the kind of capability
    pub kind: String,

    /// Information about the capability
    pub description: String,
}

impl Capability {
    /// Will convert the [`Capability`]'s `kind` into a known [`CapabilityKind`] if possible,
    /// returning None if the capability is unknown
    pub fn to_capability_kind(&self) -> Option<CapabilityKind> {
        CapabilityKind::from_str(&self.kind).ok()
    }

    /// Returns true if the described capability is unknown
    pub fn is_unknown(&self) -> bool {
        self.to_capability_kind().is_none()
    }
}

impl PartialEq for Capability {
    fn eq(&self, other: &Self) -> bool {
        self.kind.eq_ignore_ascii_case(&other.kind)
    }
}

impl Eq for Capability {}

impl PartialOrd for Capability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Capability {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .to_ascii_lowercase()
            .cmp(&other.kind.to_ascii_lowercase())
    }
}

impl Hash for Capability {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.to_ascii_lowercase().hash(state);
    }
}

impl From<CapabilityKind> for Capability {
    /// Creates a new capability using the kind's default message
    fn from(kind: CapabilityKind) -> Self {
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
impl Capability {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Capability)
    }
}

#[cfg(feature = "schemars")]
impl CapabilityKind {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(CapabilityKind)
    }
}
