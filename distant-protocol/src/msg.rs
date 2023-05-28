use derive_more::From;
use serde::{Deserialize, Serialize};

/// Represents a wrapper around a distant message, supporting single and batch requests
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Msg<T> {
    Single(T),
    Batch(Vec<T>),
}

impl<T> Msg<T> {
    /// Returns true if msg has a single payload
    pub fn is_single(&self) -> bool {
        matches!(self, Self::Single(_))
    }

    /// Returns reference to single value if msg is single variant
    pub fn as_single(&self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to single value if msg is single variant
    pub fn as_mut_single(&mut self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the single value if msg is single variant
    pub fn into_single(self) -> Option<T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns true if msg has a batch of payloads
    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch(_))
    }

    /// Returns reference to batch value if msg is batch variant
    pub fn as_batch(&self) -> Option<&[T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to batch value if msg is batch variant
    pub fn as_mut_batch(&mut self) -> Option<&mut [T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the batch value if msg is batch variant
    pub fn into_batch(self) -> Option<Vec<T>> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Convert into a collection of payload data
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::Single(x) => vec![x],
            Self::Batch(x) => x,
        }
    }
}
