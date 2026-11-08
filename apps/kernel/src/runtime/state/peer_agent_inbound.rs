//! Peers across kernels, receiving side: a same-owner kernel's agent messages, steers or cancels
//! an ordinary agent of a room here. Admission (owner, stamp) happened at the relay seam.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

impl KernelRuntimeState {
    /// `kernel` is the authenticated sending kernel, `owner` the owner both kernels share.
    pub(crate) async fn receive_peer_agent_request(
        &self,
        _kernel: &str,
        _owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match request {
            RelayPeerRequest::PeerAgentMessage { .. }
            | RelayPeerRequest::PeerAgentCancel { .. }
            | RelayPeerRequest::SubscribePeerAgent { .. }
            | RelayPeerRequest::UnsubscribePeerAgent { .. }
            | RelayPeerRequest::SpawnRoomPeer { .. }
            | RelayPeerRequest::DeliverPeerAgentOutcome { .. } => Err(
                crate::durable_state::agent_lifecycle::error(
                    "this kernel does not carry peer agent requests across kernels yet",
                ),
            ),
            _ => Err(crate::durable_state::agent_lifecycle::error(
                "not a peer agent request",
            )),
        }
    }
}
