//! This carrier is confined to authenticated, encrypted kernel peers.
use super::{AgentMoveArtifactKind, AgentMoveCapture, AgentMoveIdentity, AgentMoveMaterialization};
use crate::transport::relay_peer::RemoteSkillSyncContext;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMoveChunk {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for AgentMoveChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentMoveChunk")
            .field("offset", &self.offset)
            .field("length", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMovePeerRequest {
    Probe {
        identity: AgentMoveIdentity,
    },
    ProbeSource {
        identity: AgentMoveIdentity,
    },
    PrepareSource {
        identity: AgentMoveIdentity,
        context: RemoteSkillSyncContext,
    },
    ReadChunk {
        identity: AgentMoveIdentity,
        kind: AgentMoveArtifactKind,
        offset: u64,
    },
    BeginTarget {
        capture: AgentMoveCapture,
    },
    WriteChunk {
        identity: AgentMoveIdentity,
        kind: AgentMoveArtifactKind,
        chunk: AgentMoveChunk,
    },
    PublishTarget {
        identity: AgentMoveIdentity,
    },
    AttachTarget {
        identity: AgentMoveIdentity,
        context: RemoteSkillSyncContext,
    },
    ActivateTarget {
        identity: AgentMoveIdentity,
    },
    RetireSource {
        identity: AgentMoveIdentity,
    },
    ReleaseSource {
        identity: AgentMoveIdentity,
    },
}

impl AgentMovePeerRequest {
    pub fn identity(&self) -> &AgentMoveIdentity {
        match self {
            Self::Probe { identity }
            | Self::ProbeSource { identity }
            | Self::PrepareSource { identity, .. }
            | Self::ReadChunk { identity, .. }
            | Self::WriteChunk { identity, .. }
            | Self::PublishTarget { identity }
            | Self::AttachTarget { identity, .. }
            | Self::ActivateTarget { identity }
            | Self::RetireSource { identity }
            | Self::ReleaseSource { identity } => identity,
            Self::BeginTarget { capture } => &capture.identity,
        }
    }
    pub(crate) fn leased_agent_id(&self) -> Option<&str> {
        match self {
            Self::PrepareSource { context, .. } | Self::AttachTarget { context, .. } => {
                Some(&context.leased_agent_id)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMovePeerResponse {
    Compatible,
    SourcePrepared { capture: AgentMoveCapture },
    Chunk { chunk: AgentMoveChunk },
    TargetPrepared,
    ChunkWritten,
    Published { receipt: AgentMoveMaterialization },
    Attached,
    AttachmentMissing,
    Activated,
    SourceRetired,
    SourceReleased,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn mk03_private_move_peer_214_shape_and_hash() {
        assert_eq!(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let identity: AgentMoveIdentity = serde_json::from_value(serde_json::json!({"operation_id": "move-1", "generation": 1, "home_kernel_id": "home", "home_session_id": "room", "home_agent_id": "agent", "owner_user_id": "owner", "source_kernel_id": "source", "target_kernel_id": "target", "home_key_thumbprint": "home-key", "source_key_thumbprint": "source-key", "target_key_thumbprint": "target-key", "target_public_key": "target-public"})).unwrap();
        let request = crate::transport::relay_peer::RelayPeerRequest::AgentMove {
            request: AgentMovePeerRequest::WriteChunk {
                identity,
                kind: AgentMoveArtifactKind::Workspace,
                chunk: AgentMoveChunk {
                    offset: 0,
                    bytes: vec![1, 2, 3],
                },
            },
        };
        let response = crate::transport::relay_peer::RelayPeerResponse::AgentMove {
            response: AgentMovePeerResponse::ChunkWritten,
        };
        let value = serde_json::json!({"request": request, "response": response});
        assert_eq!(value["request"]["kind"], "agent_move");
        assert_eq!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
            "6090d29be16ce4a437a4884ad5aed02a794b4cc24c7080cc0105c066c19bba54"
        );
        let parsed: crate::transport::relay_peer::RelayPeerRequest =
            serde_json::from_value(value["request"].clone()).unwrap();
        assert!(matches!(
            parsed,
            crate::transport::relay_peer::RelayPeerRequest::AgentMove { .. }
        ));
        let debug = format!(
            "{:?}",
            AgentMoveChunk {
                offset: 0,
                bytes: b"private-native-history".to_vec()
            }
        );
        assert!(!debug.contains("private-native-history"));
        assert!(!debug.contains("112, 114"));
    }
}
