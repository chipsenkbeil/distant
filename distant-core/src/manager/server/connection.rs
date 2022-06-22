use crate::{
    manager::data::{Destination, Extra},
    DistantClient,
};

/// Represents a connection a distant manager has with some distant-compatible server
pub struct DistantManagerConnection {
    pub id: usize,
    pub destination: Destination,
    pub extra: Extra,
    pub client: DistantClient,
}
