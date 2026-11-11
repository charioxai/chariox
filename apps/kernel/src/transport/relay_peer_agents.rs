//! Peers across kernels of one owner: the wire shapes an agent on one kernel uses to message,
//! steer, cancel, follow or spawn an ordinary agent in a room of another kernel of its owner.
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use serde::{Deserialize, Serialize};

/// Who acts, as the sending kernel fills it. The receiving kernel checks `kernel_id` against
/// the authenticated link before it trusts the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAgentStamp {
    pub kernel_id: String,
    pub room_id: String,
    pub agent_id: String,
}

/// An agent of a room on the receiving kernel: id, ref or alias, in a room id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAgentAddress {
    pub room: String,
    pub agent: String,
}

/// The caller's effective authority on its own kernel. The receiving kernel lowers it to its
/// own record of the target room and never trusts more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCallerAuthority {
    pub mode: AgentExecutionMode,
    pub permission_level: AgentPermissionLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerFollow {
    pub subscription_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerEventKind {
    TurnEnded,
    TurnFailed,
    SourceLost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeerAgentOutcome {
    Event {
        subscription_id: String,
        source: PeerAgentStamp,
        occurrence_id: String,
        event: PeerEventKind,
    },
    /// The answer to a message the receiving kernel's agent sent with a reply requested.
    Reply {
        request_id: String,
        source: PeerAgentStamp,
        occurrence_id: String,
        success: bool,
        public_answer: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerOutcomeAck {
    Accepted,
    Duplicate,
    UnknownSubscription,
}

/// Why the receiving kernel did not act. `RoomUnavailable` covers an unknown room, another
/// owner's room and a hidden room alike, so a caller learns nothing it cannot address.
/// `kernel_unreachable`, `uncertain` and `owner_unverified` never cross the wire: the sending
/// kernel decides them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PeerAgentRefusal {
    RoomUnavailable,
    AgentUnavailable,
    Ambiguous,
    MoreAuthority {
        mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    },
    SubAgentCaller,
    RoomFull,
    AttachmentsLocalOnly,
}

#[cfg(test)]
mod shapes {
    use super::*;
    use crate::transport::relay_peer::{
        RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
    };

    /// Every variant of the cross-kernel peer link, pinned in `fixtures/peer-agent-link-protocol.json`
    /// with its protocol number zeroed, so a shape change is a deliberate fixture change.
    #[test]
    fn peer_agent_link_shapes_match_the_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/peer-agent-link-protocol.json"
        ))
        .unwrap();
        assert_eq!(fixture["peer"], RELAY_PEER_PROTOCOL_VERSION);
        let a = PeerAgentStamp {
            kernel_id: "kernel-a".into(),
            room_id: "room-a".into(),
            agent_id: "agent-a".into(),
        };
        let a2 = a.clone();
        let b = PeerAgentStamp {
            kernel_id: "kernel-b".into(),
            room_id: "room-b".into(),
            agent_id: "agent-b".into(),
        };
        let target = PeerAgentAddress {
            room: "room-b".into(),
            agent: "peer".into(),
        };
        let authority = PeerCallerAuthority {
            mode: AgentExecutionMode::Build,
            permission_level: AgentPermissionLevel::Required,
        };
        let v = RELAY_PEER_PROTOCOL_VERSION;
        let requests = vec![
            RelayPeerRequest::PeerAgentMessage {
                protocol_version: v,
                request_id: "request-1".into(),
                sender: a.clone(),
                target: target.clone(),
                message: "parser.rs is mine".into(),
                urgent: true,
                reply_requested: false,
                caller_authority: authority,
            },
            RelayPeerRequest::PeerAgentCancel {
                protocol_version: v,
                request_id: "request-2".into(),
                sender: a.clone(),
                target: target.clone(),
                caller_authority: authority,
            },
            RelayPeerRequest::SubscribePeerAgent {
                protocol_version: v,
                request_id: "request-3".into(),
                sender: a.clone(),
                target,
                subscription_id: "follow-1".into(),
                generation: 2,
            },
            RelayPeerRequest::UnsubscribePeerAgent {
                protocol_version: v,
                subscription_id: "follow-1".into(),
                generation: 3,
            },
            RelayPeerRequest::SpawnRoomPeer {
                protocol_version: v,
                request_id: "request-4".into(),
                sender: a,
                room: "room-b".into(),
                provider: "claude".into(),
                account_profile: None,
                model: Some("haiku".into()),
                effort: Some("low".into()),
                alias: Some("reviewer".into()),
                prompt: Some("review parser.rs".into()),
                execution_mode: Some(AgentExecutionMode::Plan),
                permission_level: Some(AgentPermissionLevel::Required),
                caller_authority: authority,
                follow: Some(PeerFollow {
                    subscription_id: "follow-2".into(),
                    generation: 1,
                }),
            },
            RelayPeerRequest::DeleteRoomPeer {
                protocol_version: v,
                request_id: "request-4".into(),
                sender: a2,
            },
            RelayPeerRequest::DeliverPeerAgentOutcome {
                protocol_version: v,
                delivery_id: "delivery-1".into(),
                outcome: PeerAgentOutcome::Event {
                    subscription_id: "follow-1".into(),
                    source: b.clone(),
                    occurrence_id: "turn-7".into(),
                    event: PeerEventKind::TurnEnded,
                },
            },
            RelayPeerRequest::DeliverPeerAgentOutcome {
                protocol_version: v,
                delivery_id: "delivery-2".into(),
                outcome: PeerAgentOutcome::Reply {
                    request_id: "request-1".into(),
                    source: b.clone(),
                    occurrence_id: "turn-8".into(),
                    success: true,
                    public_answer: Some(serde_json::json!({"text": "done"})),
                },
            },
        ];
        let responses = vec![
            RelayPeerResponse::PeerAgentAccepted {
                protocol_version: v,
                duplicate: false,
                target: b.clone(),
            },
            RelayPeerResponse::PeerAgentSubscribed {
                protocol_version: v,
                subscription_id: "follow-1".into(),
                generation: 2,
                target: b.clone(),
            },
            RelayPeerResponse::RoomPeerSpawned {
                protocol_version: v,
                duplicate: true,
                agent: b,
                alias: "reviewer".into(),
            },
            RelayPeerResponse::RoomPeerDeleted {
                protocol_version: v,
                duplicate: true,
            },
            RelayPeerResponse::PeerAgentOutcomeAck {
                protocol_version: v,
                ack: PeerOutcomeAck::UnknownSubscription,
            },
            RelayPeerResponse::PeerAgentRefused {
                protocol_version: v,
                refusal: PeerAgentRefusal::MoreAuthority {
                    mode: AgentExecutionMode::Build,
                    permission_level: AgentPermissionLevel::Yolo,
                },
            },
            RelayPeerResponse::PeerAgentRefused {
                protocol_version: v,
                refusal: PeerAgentRefusal::RoomUnavailable,
            },
        ];
        for request in &requests {
            let wire = serde_json::to_value(request).unwrap();
            assert_eq!(
                &serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
                request
            );
        }
        for response in &responses {
            let wire = serde_json::to_value(response).unwrap();
            assert_eq!(
                &serde_json::from_value::<RelayPeerResponse>(wire).unwrap(),
                response
            );
        }
        let mut wire = serde_json::json!({"requests": requests, "responses": responses});
        for collection in ["requests", "responses"] {
            for message in wire[collection].as_array_mut().unwrap() {
                message["protocol_version"] = 0.into();
            }
        }
        assert_eq!(wire["requests"], fixture["requests"]);
        assert_eq!(wire["responses"], fixture["responses"]);
    }
}
