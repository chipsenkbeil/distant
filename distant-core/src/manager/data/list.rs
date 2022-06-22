use super::Destination;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

/// Represents a list of information about active connections
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionList(pub(crate) HashMap<usize, Destination>);

impl ConnectionList {
    /// Returns a reference to the destination associated with an active connection
    pub fn connection_destination(&self, id: usize) -> Option<&Destination> {
        self.0.get(&id)
    }
}

impl Deref for ConnectionList {
    type Target = HashMap<usize, Destination>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ConnectionList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
