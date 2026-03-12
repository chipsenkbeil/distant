use crate::auth::msg::AuthenticationResponse;
use serde::{Deserialize, Serialize};

use super::{ManagedTunnelId, ManagerAuthenticationId, ManagerChannelId};
use crate::net::common::{ConnectionId, Map, UntypedRequest};

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Retrieve information about the manager's version.
    Version,

    /// Launch a server using the manager
    Launch {
        /// Raw destination string (e.g. `"docker://ubuntu:22.04"` or `"ssh://host:22"`).
        /// Parsing is deferred to the plugin matched by scheme.
        destination: String,

        /// Additional options specific to the connection
        options: Map,
    },

    /// Initiate a connection through the manager
    Connect {
        /// Raw destination string. Parsing is deferred to the plugin matched by scheme.
        destination: String,

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

    /// Start a forward tunnel (local listener -> remote target) in the manager
    ForwardTunnel {
        connection_id: ConnectionId,
        bind_port: u16,
        remote_host: String,
        remote_port: u16,
    },

    /// Start a reverse tunnel (remote listener -> local target) in the manager
    ReverseTunnel {
        connection_id: ConnectionId,
        remote_port: u16,
        local_host: String,
        local_port: u16,
    },

    /// Close a managed tunnel by ID
    CloseManagedTunnel { id: ManagedTunnelId },

    /// List all managed tunnels
    ListManagedTunnels,
}
