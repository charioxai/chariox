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
        native: Option<crate::provider::NativeMoveCompatibility>,
    },
    ProbeSource {
        identity: AgentMoveIdentity,
        context: RemoteSkillSyncContext,
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
    CreateLease {
        identity: AgentMoveIdentity,
        home_agent_metaagent: bool,
    },
    AbortTarget {
        identity: AgentMoveIdentity,
    },
    BeginTarget {
        capture: Box<AgentMoveCapture>,
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
        requirements: Box<super::AgentMoveRequirements>,
    },
    ActivateTarget {
        identity: AgentMoveIdentity,
        readiness: Box<super::AgentMoveReadiness>,
    },
    RetireSource {
        identity: AgentMoveIdentity,
    },
    ReleaseSource {
        identity: AgentMoveIdentity,
        allow_capture: bool,
    },
}

impl AgentMovePeerRequest {
    pub fn identity(&self) -> &AgentMoveIdentity {
        match self {
            Self::Probe { identity, .. }
            | Self::ProbeSource { identity, .. }
            | Self::PrepareSource { identity, .. }
            | Self::ReadChunk { identity, .. }
            | Self::WriteChunk { identity, .. }
            | Self::PublishTarget { identity }
            | Self::AttachTarget { identity, .. }
            | Self::ActivateTarget { identity, .. }
            | Self::RetireSource { identity }
            | Self::ReleaseSource { identity, .. }
            | Self::AbortTarget { identity }
            | Self::CreateLease { identity, .. } => identity,
            Self::BeginTarget { capture } => &capture.identity,
        }
    }
    pub(crate) fn leased_agent_id(&self) -> Option<&str> {
        match self {
            Self::ProbeSource { context, .. }
            | Self::PrepareSource { context, .. }
            | Self::AttachTarget { context, .. } => Some(&context.leased_agent_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMovePeerResponse {
    Compatible {
        relay_peer_protocol_version: u32,
    },
    SourceCompatible {
        relay_peer_protocol_version: u32,
        native: Option<crate::provider::NativeMoveCompatibility>,
    },
    SourceCaptured,
    TargetAborted,
    LeaseCreated {
        lease: crate::execution_lease::ExecutionLease,
        relay_peer_protocol_version: u32,
    },
    SourcePrepared {
        capture: Box<AgentMoveCapture>,
    },
    Chunk {
        chunk: AgentMoveChunk,
    },
    TargetPrepared,
    ChunkWritten,
    Published {
        receipt: Box<AgentMoveMaterialization>,
    },
    Attached {
        receipt: Box<super::AgentMoveReadiness>,
    },
    AttachmentMissing,
    Activated {
        receipt: Box<super::AgentMoveCommitReceipt>,
    },
    SourceRetired,
    SourceReleased,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn mk04_resource_readiness_639_241_shape_and_hash() {
        let bytes = include_bytes!("protocol-639-241.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            "d394d028997d6f30914876997131e89f4d0a4b2c2e3cbbd696c99408b1a08587"
        );
        let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(
            fixture["local_protocol"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            fixture["peer_protocol"],
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        );
        let request: crate::local::LocalDaemonRequest =
            serde_json::from_value(fixture["abort"].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), fixture["abort"]);
        for value in fixture["agent_move_statuses"].as_array().unwrap() {
            let status: crate::agent_move::AgentMoveStatus =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(status).unwrap(), *value);
        }
        for value in fixture["peer_requests"].as_array().unwrap() {
            let request: AgentMovePeerRequest = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), *value);
        }
        for value in fixture["peer_responses"].as_array().unwrap() {
            let response: AgentMovePeerResponse = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(response).unwrap(), *value);
        }
    }

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

#[cfg(test)]
mod composition_probe_tests {
    use super::AgentMovePeerResponse;
    #[test]
    fn mk04_probe_receipts_require_an_explicit_peer_version() {
        for value in [
            serde_json::json!({"result":"compatible"}),
            serde_json::json!({"result":"source_compatible","native":null}),
        ] {
            assert!(
                serde_json::from_value::<AgentMovePeerResponse>(value).is_err(),
                "A legacy Probe cannot prove composed Move compatibility"
            );
        }
    }
}
