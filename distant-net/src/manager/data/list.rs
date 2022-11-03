use crate::common::{ConnectionId, Destination};
use derive_more::IntoIterator;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut, Index, IndexMut},
};

/// Represents a list of information about active connections
#[derive(Clone, Debug, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
pub struct ConnectionList(pub(crate) HashMap<ConnectionId, Destination>);

impl ConnectionList {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Returns a reference to the destination associated with an active connection
    pub fn connection_destination(&self, id: ConnectionId) -> Option<&Destination> {
        self.0.get(&id)
    }
}

impl Default for ConnectionList {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for ConnectionList {
    type Target = HashMap<ConnectionId, Destination>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ConnectionList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Index<ConnectionId> for ConnectionList {
    type Output = Destination;

    fn index(&self, connection_id: ConnectionId) -> &Self::Output {
        &self.0[&connection_id]
    }
}

impl IndexMut<ConnectionId> for ConnectionList {
    fn index_mut(&mut self, connection_id: ConnectionId) -> &mut Self::Output {
        self.0
            .get_mut(&connection_id)
            .expect("No connection with id")
    }
}
