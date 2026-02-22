use std::collections::HashMap;
use std::ops::{Deref, DerefMut, Index, IndexMut};

use derive_more::IntoIterator;
use serde::{Deserialize, Serialize};

use crate::net::common::{ConnectionId, Destination};

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

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::net::common::Host;

    fn make_destination(name: &str) -> Destination {
        Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name(name.to_string()),
            port: Some(22),
        }
    }

    #[test]
    fn new_should_create_empty_list() {
        let list = ConnectionList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn default_should_create_empty_list() {
        let list = ConnectionList::default();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn connection_destination_should_return_some_for_existing_id() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.insert(1, dest.clone());

        let result = list.connection_destination(1);
        assert_eq!(result, Some(&dest));
    }

    #[test]
    fn connection_destination_should_return_none_for_missing_id() {
        let list = ConnectionList::new();
        assert_eq!(list.connection_destination(999), None);
    }

    #[test]
    fn deref_should_expose_underlying_hashmap() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.0.insert(1, dest.clone());

        // Deref lets us call HashMap methods directly
        assert!(list.contains_key(&1));
        assert!(!list.contains_key(&2));
        assert_eq!(list.get(&1), Some(&dest));
    }

    #[test]
    fn deref_mut_should_allow_mutation_through_hashmap_methods() {
        let dest1 = make_destination("host1");
        let dest2 = make_destination("host2");
        let mut list = ConnectionList::new();

        list.insert(1, dest1.clone());
        list.insert(2, dest2.clone());

        assert_eq!(list.len(), 2);

        list.remove(&1);
        assert_eq!(list.len(), 1);
        assert!(!list.contains_key(&1));
        assert!(list.contains_key(&2));
    }

    #[test]
    fn index_should_return_destination_for_existing_id() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.insert(1, dest.clone());

        assert_eq!(list[1], dest);
    }

    #[test]
    #[should_panic]
    fn index_should_panic_for_missing_id() {
        let list = ConnectionList::new();
        let _ = &list[999];
    }

    #[test]
    fn index_mut_should_allow_mutation_for_existing_id() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.insert(1, dest);

        let dest_mut = &mut list[1];
        dest_mut.port = Some(8080);

        assert_eq!(list[1].port, Some(8080));
    }

    #[test]
    #[should_panic(expected = "No connection with id")]
    fn index_mut_should_panic_for_missing_id() {
        let mut list = ConnectionList::new();
        list[999] = make_destination("host1");
    }

    #[test]
    fn into_iterator_should_yield_all_entries() {
        let dest1 = make_destination("host1");
        let dest2 = make_destination("host2");
        let mut list = ConnectionList::new();
        list.insert(1, dest1.clone());
        list.insert(2, dest2.clone());

        let collected: HashMap<ConnectionId, Destination> = list.into_iter().collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[&1], dest1);
        assert_eq!(collected[&2], dest2);
    }

    #[test]
    fn clone_should_produce_equal_list() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.insert(1, dest);

        let cloned = list.clone();
        assert_eq!(list, cloned);
    }

    #[test]
    fn serialize_and_deserialize_should_round_trip() {
        let dest = make_destination("host1");
        let mut list = ConnectionList::new();
        list.insert(42, dest);

        let json = serde_json::to_string(&list).unwrap();
        let deserialized: ConnectionList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn serialize_empty_list_should_round_trip() {
        let list = ConnectionList::new();
        let json = serde_json::to_string(&list).unwrap();
        let deserialized: ConnectionList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn serialize_multiple_entries_should_round_trip() {
        let mut list = ConnectionList::new();
        list.insert(1, make_destination("host1"));
        list.insert(2, make_destination("host2"));
        list.insert(3, make_destination("host3"));

        let json = serde_json::to_string(&list).unwrap();
        let deserialized: ConnectionList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, deserialized);
    }

    #[test]
    fn connection_destination_should_work_with_multiple_entries() {
        let dest1 = make_destination("host1");
        let dest2 = make_destination("host2");
        let mut list = ConnectionList::new();
        list.insert(1, dest1.clone());
        list.insert(2, dest2.clone());

        assert_eq!(list.connection_destination(1), Some(&dest1));
        assert_eq!(list.connection_destination(2), Some(&dest2));
        assert_eq!(list.connection_destination(3), None);
    }

    #[test]
    fn debug_format_should_not_panic() {
        let mut list = ConnectionList::new();
        list.insert(1, make_destination("host1"));
        let debug_str = format!("{:?}", list);
        assert!(!debug_str.is_empty());
    }
}
