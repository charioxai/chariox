//! Peers across kernels, receiving side: a request from a same-owner kernel reaches an
//! ordinary agent of a room here exactly as a peer in another room does.
use super::*;
use crate::provider::AgentExecutionMode;
use crate::transport::relay_peer::{
    PeerAgentAddress, PeerAgentRefusal, PeerAgentStamp, PeerCallerAuthority, RelayPeerRequest,
    RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

const KERNEL_A: &str = "kernel-a";

fn stamp() -> PeerAgentStamp {
    PeerAgentStamp {
        kernel_id: KERNEL_A.into(),
        room_id: "room-on-a".into(),
        agent_id: "agent-on-a".into(),
    }
}

fn authority(mode: AgentExecutionMode, level: AgentPermissionLevel) -> PeerCallerAuthority {
    PeerCallerAuthority {
        mode,
        permission_level: level,
    }
}

fn message(rooms: &Rooms, request_id: &str, agent: &str, urgent: bool) -> RelayPeerRequest {
    message_as(rooms, request_id, agent, urgent, AgentPermissionLevel::Yolo)
}

fn message_as(
    rooms: &Rooms,
    request_id: &str,
    agent: &str,
    urgent: bool,
    level: AgentPermissionLevel,
) -> RelayPeerRequest {
    RelayPeerRequest::PeerAgentMessage {
        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        request_id: request_id.into(),
        sender: stamp(),
        target: PeerAgentAddress {
            room: rooms.peer_room.clone(),
            agent: agent.into(),
        },
        message: "parser.rs is mine for the next hour".into(),
        urgent,
        reply_requested: false,
        caller_authority: authority(AgentExecutionMode::Build, level),
    }
}

impl Rooms {
    async fn from_kernel_a(&self, request: RelayPeerRequest) -> RelayPeerResponse {
        let owner = self.peer.owner_user_id().to_string();
        self.router
            .relay_peer_agent(KERNEL_A, &owner, request)
            .await
            .expect("the peer request answers")
    }
}

fn refusal(response: RelayPeerResponse) -> PeerAgentRefusal {
    match response {
        RelayPeerResponse::PeerAgentRefused { refusal, .. } => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_message_from_another_kernel_lands_once_as_a_stamped_message() {
    run_peer_test("peer-kernel-message", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Required);
        for _ in 0..2 {
            let response = rooms
                .from_kernel_a(message(&rooms, "request-1", "peer", false))
                .await;
            assert!(
                matches!(&response, RelayPeerResponse::PeerAgentAccepted { target, .. } if target.agent_id == rooms.peer.id()),
                "{response:?}"
            );
        }
        let inbox = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(inbox.len(), 1, "one request id is one delivery");
        assert!(!inbox[0].urgent);
        assert_eq!(inbox[0].payload["sender_kernel_id"], KERNEL_A);
        assert_eq!(inbox[0].payload["sender_agent_id"], "agent-on-a");
        let RelayPeerResponse::PeerAgentAccepted { duplicate, .. } = rooms
            .from_kernel_a(message(&rooms, "request-1", "peer", false))
            .await
        else {
            panic!("accepted")
        };
        assert!(duplicate);
    });
}

#[test]
fn a_peer_on_another_kernel_addresses_only_ordinary_agents_of_its_owner() {
    run_peer_test("peer-kernel-address", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let hidden = rooms
            .from_kernel_a(message(&rooms, "r-hidden", "hidden", false))
            .await;
        assert_eq!(refusal(hidden), PeerAgentRefusal::AgentUnavailable);
        let mut foreign = message(&rooms, "r-foreign", "foreign", false);
        if let RelayPeerRequest::PeerAgentMessage { target, .. } = &mut foreign {
            target.room = rooms.foreign_room.clone();
        }
        assert_eq!(
            refusal(rooms.from_kernel_a(foreign).await),
            PeerAgentRefusal::RoomUnavailable
        );
        let mut missing = message(&rooms, "r-missing", "peer", false);
        if let RelayPeerRequest::PeerAgentMessage { target, .. } = &mut missing {
            target.room = "no-such-room".into();
        }
        assert_eq!(
            refusal(rooms.from_kernel_a(missing).await),
            PeerAgentRefusal::RoomUnavailable
        );
        assert!(rooms
            .inbox(&rooms.foreign_room, rooms.foreign.id())
            .await
            .is_empty());
        assert!(rooms
            .inbox(&rooms.peer_room, rooms.hidden.id())
            .await
            .is_empty());
    });
}

#[test]
fn a_steer_from_another_kernel_needs_at_least_the_targets_authority() {
    run_peer_test("peer-kernel-steer", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Required, AgentPermissionLevel::Yolo);
        let response = rooms
            .from_kernel_a(message_as(
                &rooms,
                "steer-1",
                "peer",
                true,
                AgentPermissionLevel::Required,
            ))
            .await;
        assert!(
            matches!(
                refusal(response),
                PeerAgentRefusal::MoreAuthority {
                    permission_level: AgentPermissionLevel::Yolo,
                    ..
                }
            ),
            "a Required caller cannot steer a Yolo peer"
        );
        assert!(rooms
            .inbox(&rooms.peer_room, rooms.peer.id())
            .await
            .is_empty());
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
    });
}
