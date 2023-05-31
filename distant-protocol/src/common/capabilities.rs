use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::{BitAnd, BitOr, BitXor, Deref, DerefMut};
use std::str::FromStr;

use derive_more::{From, Into, IntoIterator};
use serde::{Deserialize, Serialize};
use strum::{EnumMessage, IntoEnumIterator};

/// Represents the kinds of capabilities available.
pub use crate::request::RequestKind as CapabilityKind;

/// Set of supported capabilities for a server
#[derive(Clone, Debug, From, Into, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
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

impl AsRef<HashSet<Capability>> for Capabilities {
    fn as_ref(&self) -> &HashSet<Capability> {
        &self.0
    }
}

impl AsMut<HashSet<Capability>> for Capabilities {
    fn as_mut(&mut self) -> &mut HashSet<Capability> {
        &mut self.0
    }
}

impl Deref for Capabilities {
    type Target = HashSet<Capability>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Capabilities {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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

#[cfg(test)]
mod tests {
    use super::*;

    mod capabilities {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let capabilities: Capabilities = [Capability {
                kind: "some kind".to_string(),
                description: "some description".to_string(),
            }]
            .into_iter()
            .collect();

            let value = serde_json::to_value(capabilities).unwrap();
            assert_eq!(
                value,
                serde_json::json!([
                    {
                        "kind": "some kind",
                        "description": "some description",
                    }
                ])
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!([
                {
                    "kind": "some kind",
                    "description": "some description",
                }
            ]);

            let capabilities: Capabilities = serde_json::from_value(value).unwrap();
            assert_eq!(
                capabilities,
                [Capability {
                    kind: "some kind".to_string(),
                    description: "some description".to_string(),
                }]
                .into_iter()
                .collect()
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let capabilities: Capabilities = [Capability {
                kind: "some kind".to_string(),
                description: "some description".to_string(),
            }]
            .into_iter()
            .collect();

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&capabilities).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(
                &[Capability {
                    kind: "some kind".to_string(),
                    description: "some description".to_string(),
                }]
                .into_iter()
                .collect::<Capabilities>(),
            )
            .unwrap();

            let capabilities: Capabilities = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                capabilities,
                [Capability {
                    kind: "some kind".to_string(),
                    description: "some description".to_string(),
                }]
                .into_iter()
                .collect()
            );
        }
    }

    mod capability {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let capability = Capability {
                kind: "some kind".to_string(),
                description: "some description".to_string(),
            };

            let value = serde_json::to_value(capability).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "kind": "some kind",
                    "description": "some description",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "kind": "some kind",
                "description": "some description",
            });

            let capability: Capability = serde_json::from_value(value).unwrap();
            assert_eq!(
                capability,
                Capability {
                    kind: "some kind".to_string(),
                    description: "some description".to_string(),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let capability = Capability {
                kind: "some kind".to_string(),
                description: "some description".to_string(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&capability).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Capability {
                kind: "some kind".to_string(),
                description: "some description".to_string(),
            })
            .unwrap();

            let capability: Capability = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                capability,
                Capability {
                    kind: "some kind".to_string(),
                    description: "some description".to_string(),
                }
            );
        }
    }
}
