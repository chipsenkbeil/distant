use distant_auth::msg::AuthenticationResponse;
use serde::{Deserialize, Serialize};

use super::{ManagerAuthenticationId, ManagerChannelId};
use crate::common::{ConnectionId, Destination, Map, UntypedRequest};

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Retrieve information about the manager's version.
    Version,

    /// Launch a server using the manager
    Launch {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        options: Map,
    },

    /// Initiate a connection through the manager
    Connect {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        options: Map,
    },

    /// Submit some authentication message for the manager to use with an active connection
    Authenticate {
        /// Id of the authentication request that is being responded to
        id: ManagerAuthenticationId,

        /// Response being sent to some active connection
        msg: AuthenticationResponse,
    },

    /// Opens a channel for communication with an already-connected server
    OpenChannel {
        /// Id of the connection
        id: ConnectionId,
    },

    /// Sends data through channel
    Channel {
        /// Id of the channel
        id: ManagerChannelId,

        /// Untyped request to send through the channel
        request: UntypedRequest<'static>,
    },

    /// Closes an open channel
    CloseChannel {
        /// Id of the channel to close
        id: ManagerChannelId,
    },

    /// Retrieve information about a specific connection
    Info { id: ConnectionId },

    /// Kill a specific connection
    Kill { id: ConnectionId },

    /// Retrieve list of connections being managed
    List,
}
