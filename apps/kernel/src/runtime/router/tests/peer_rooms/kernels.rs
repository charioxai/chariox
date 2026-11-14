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

fn cancel(rooms: &Rooms, mode: AgentExecutionMode) -> RelayPeerRequest {
    RelayPeerRequest::PeerAgentCancel {
        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        request_id: "cancel-1".into(),
        sender: stamp(),
        target: PeerAgentAddress {
            room: rooms.peer_room.clone(),
            agent: "peer".into(),
        },
        caller_authority: authority(mode, AgentPermissionLevel::Required),
    }
}

impl Rooms {
    async fn request_from_kernel_a(&self, request: RelayPeerRequest) -> RelayPeerResponse {
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
                .request_from_kernel_a(message(&rooms, "request-1", "peer", false))
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
            .request_from_kernel_a(message(&rooms, "request-1", "peer", false))
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
            .request_from_kernel_a(message(&rooms, "r-hidden", "hidden", false))
            .await;
        assert_eq!(refusal(hidden), PeerAgentRefusal::AgentUnavailable);
        let mut foreign = message(&rooms, "r-foreign", "foreign", false);
        if let RelayPeerRequest::PeerAgentMessage { target, .. } = &mut foreign {
            target.room = rooms.foreign_room.clone();
        }
        assert_eq!(
            refusal(rooms.request_from_kernel_a(foreign).await),
            PeerAgentRefusal::RoomUnavailable
        );
        let mut missing = message(&rooms, "r-missing", "peer", false);
        if let RelayPeerRequest::PeerAgentMessage { target, .. } = &mut missing {
            target.room = "no-such-room".into();
        }
        assert_eq!(
            refusal(rooms.request_from_kernel_a(missing).await),
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
            .request_from_kernel_a(message_as(
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

#[test]
fn a_peer_on_another_kernel_steers_and_cancels_with_equal_authority() {
    run_peer_test("peer-kernel-control", || async {
        let rooms = Rooms::new(
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Required,
        );
        let steer = rooms
            .request_from_kernel_a(message_as(
                &rooms,
                "steer-2",
                "peer",
                true,
                AgentPermissionLevel::Required,
            ))
            .await;
        assert!(
            matches!(steer, RelayPeerResponse::PeerAgentAccepted { .. }),
            "{steer:?}"
        );
        let inbox = rooms.inbox(&rooms.peer_room, rooms.peer.id()).await;
        assert_eq!(inbox.len(), 1);
        assert!(
            inbox[0].urgent,
            "a steer arrives as an urgent message, not a prompt"
        );

        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        let less = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Plan))
            .await;
        assert!(matches!(
            refusal(less),
            PeerAgentRefusal::MoreAuthority { .. }
        ));
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        let equal = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(equal, RelayPeerResponse::PeerAgentAccepted { .. }),
            "{equal:?}"
        );
        assert!(!rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
    });
}

#[test]
fn a_retried_cancel_after_a_lost_answer_returns_its_receipt_and_spares_a_later_turn() {
    run_peer_test("peer-kernel-cancel-retry", || async {
        let rooms = Rooms::new(
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Required,
        );
        let first = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(
                first,
                RelayPeerResponse::PeerAgentAccepted {
                    duplicate: false,
                    ..
                }
            ),
            "{first:?}"
        );
        assert!(!rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        // The answer to kernel A was lost; meanwhile the cancelled turn settles and the peer
        // starts its next turn.
        rooms.settle_cancelled_turn().await;
        crate::test_support::admit_room_test_turn(
            &mut *rooms.app.lock().await,
            &rooms.peer_room,
            rooms.peer.id(),
        );
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        let retry = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(
                retry,
                RelayPeerResponse::PeerAgentAccepted {
                    duplicate: true,
                    ..
                }
            ),
            "the retry answers the first cancel's receipt: {retry:?}"
        );
        assert!(
            rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await,
            "a retried cancel never reaches a later turn"
        );
    });
}

#[test]
fn a_message_from_another_kernel_after_its_cancel_runs_on_the_cancelled_agent() {
    run_peer_test("peer-kernel-cancel-then-message", || async {
        let rooms = Rooms::new(
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Required,
        );
        let cancelled = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(cancelled, RelayPeerResponse::PeerAgentAccepted { .. }),
            "{cancelled:?}"
        );
        rooms.settle_cancelled_turn().await;
        let later = rooms
            .request_from_kernel_a(message(&rooms, "after-cancel", "peer", true))
            .await;
        assert!(
            matches!(later, RelayPeerResponse::PeerAgentAccepted { .. }),
            "{later:?}"
        );
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, rooms.peer.id())
                .await,
            "a cancel ends the current task, not the agent: the later message runs"
        );
    });
}

fn spawn(request_id: &str, room: &str) -> RelayPeerRequest {
    RelayPeerRequest::SpawnRoomPeer {
        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        request_id: request_id.into(),
        sender: stamp(),
        room: room.into(),
        provider: "dev-stub".into(),
        account_profile: None,
        model: None,
        effort: None,
        alias: Some(format!("remote-{request_id}")),
        prompt: Some("review parser.rs".into()),
        execution_mode: None,
        permission_level: Some(AgentPermissionLevel::Yolo),
        caller_authority: authority(AgentExecutionMode::Build, AgentPermissionLevel::Required),
        follow: None,
    }
}

#[test]
fn a_room_agent_on_another_kernel_spawns_one_ordinary_clamped_peer_per_request() {
    run_peer_test("peer-kernel-spawn", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let first = rooms
            .request_from_kernel_a(spawn("spawn-1", &rooms.peer_room))
            .await;
        let RelayPeerResponse::RoomPeerSpawned {
            duplicate: false,
            agent,
            alias,
            ..
        } = first
        else {
            panic!("spawned: {first:?}")
        };
        assert_eq!(alias, "remote-spawn-1");
        assert_eq!(agent.room_id, rooms.peer_room);
        let retry = rooms
            .request_from_kernel_a(spawn("spawn-1", &rooms.peer_room))
            .await;
        assert!(
            matches!(&retry, RelayPeerResponse::RoomPeerSpawned { duplicate: true, agent: again, .. } if again.agent_id == agent.agent_id),
            "a retried request id is the same agent: {retry:?}"
        );
        let app = rooms.app.lock().await;
        let spawned: Vec<_> = app
            .agents()
            .get_session_agents(&rooms.peer_room)
            .into_iter()
            .filter(|candidate| candidate.alias() == Some("remote-spawn-1"))
            .collect();
        assert_eq!(spawned.len(), 1, "one agent for two sends");
        assert!(
            spawned[0].parent_agent_id().is_none(),
            "an ordinary agent, no edge"
        );
        assert_eq!(
            spawned[0].permission_level_override(),
            Some(AgentPermissionLevel::Required),
            "clamped to the caller's authority"
        );
        drop(app);
        let inbox = rooms.inbox(&rooms.peer_room, &agent.agent_id).await;
        assert_eq!(inbox.len(), 1, "the task arrives once, as a message");
        assert_eq!(inbox[0].payload["sender_kernel_id"], KERNEL_A);
    });
}

/// A spawn that names no alias: nothing but its request id tells two attempts apart.
fn unnamed_spawn(request_id: &str, room: &str) -> RelayPeerRequest {
    let mut request = spawn(request_id, room);
    if let RelayPeerRequest::SpawnRoomPeer { alias, .. } = &mut request {
        *alias = None;
    }
    request
}

fn spawned_agent(response: &RelayPeerResponse) -> String {
    match response {
        RelayPeerResponse::RoomPeerSpawned { agent, .. } => agent.agent_id.clone(),
        other => panic!("spawned: {other:?}"),
    }
}

impl Rooms {
    async fn peer_room_agents(&self) -> usize {
        self.app
            .lock()
            .await
            .agents()
            .get_session_agents(&self.peer_room)
            .len()
    }
}

#[test]
fn a_peer_spawn_retried_while_the_first_attempt_runs_creates_one_agent() {
    run_peer_test("peer-kernel-spawn-overlap", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let before = rooms.peer_room_agents().await;
        let (first, retry) = tokio::join!(
            rooms.request_from_kernel_a(unnamed_spawn("spawn-overlap", &rooms.peer_room)),
            rooms.request_from_kernel_a(unnamed_spawn("spawn-overlap", &rooms.peer_room)),
        );
        assert_eq!(spawned_agent(&first), spawned_agent(&retry));
        assert_eq!(
            rooms.peer_room_agents().await,
            before + 1,
            "two overlapping attempts of one request create one agent"
        );
        let inbox = rooms.inbox(&rooms.peer_room, &spawned_agent(&first)).await;
        assert_eq!(inbox.len(), 1, "the task arrives once");
    });
}

#[test]
fn a_peer_spawn_retried_after_its_answer_was_lost_to_a_restart_finds_its_agent() {
    run_peer_test("peer-kernel-spawn-restart", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let before = rooms.peer_room_agents().await;
        // The first attempt created the agent; the kernel stopped before it kept the answer.
        let spawn = crate::local::LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            no_subscribe: true,
            session_id: rooms.peer_room.clone(),
            alias: None,
            provider: Some("dev-stub".into()),
            account_profile: None,
            model: None,
            effort: None,
            execution_mode: Some(AgentExecutionMode::Build),
            permission_level: Some(AgentPermissionLevel::Required),
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        });
        let source = crate::runtime::command::KernelCommandSource::DaemonBackground;
        let caller = crate::runtime::command::KernelCaller {
            user_id: Some(rooms.peer.owner_user_id().into()),
            ..crate::runtime::command::KernelCaller::for_source(&source)
        };
        let key = format!("peer-spawn-{KERNEL_A}-spawn-restart");
        let mut command = KernelCommand::from_local_request_with_caller(
            key.clone(),
            source,
            caller,
            None,
            None,
            &spawn,
        );
        command.idempotency_key = Some(key);
        let Ok(crate::local::LocalDaemonResponse::AgentSpawned { agent: first, .. }) =
            rooms.router.dispatch(command, spawn).await
        else {
            panic!("the first attempt creates the agent")
        };
        let retry = rooms
            .request_from_kernel_a(unnamed_spawn("spawn-restart", &rooms.peer_room))
            .await;
        assert!(
            matches!(
                &retry,
                RelayPeerResponse::RoomPeerSpawned {
                    duplicate: true,
                    ..
                }
            ),
            "{retry:?}"
        );
        assert_eq!(spawned_agent(&retry), first.id());
        assert_eq!(rooms.peer_room_agents().await, before + 1);
        let inbox = rooms.inbox(&rooms.peer_room, first.id()).await;
        assert_eq!(inbox.len(), 1, "the task arrives once");
    });
}

#[test]
fn a_peer_spawn_from_another_kernel_into_a_room_it_cannot_address_is_refused() {
    run_peer_test("peer-kernel-spawn-refused", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        for room in ["no-such-room", rooms.foreign_room.as_str()] {
            let response = rooms.request_from_kernel_a(spawn("spawn-2", room)).await;
            assert_eq!(refusal(response), PeerAgentRefusal::RoomUnavailable);
        }
    });
}

#[test]
fn kwait_idle_cancel_from_another_kernel_stays_a_noop_on_retry() {
    run_peer_test("kwait-idle-cancel-kernel", || async {
        let rooms = Rooms::new(
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Required,
        );
        rooms
            .router
            .runtime_state
            .end_agent_turn_for_tests(&rooms.peer_room, rooms.peer.id(), true)
            .await
            .unwrap();
        assert!(
            !rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await,
            "the target must be idle before cancellation"
        );
        let response = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(
                response,
                RelayPeerResponse::PeerAgentAccepted {
                    duplicate: false,
                    ..
                }
            ),
            "{response:?}"
        );
        crate::test_support::admit_room_test_turn(
            &mut *rooms.app.lock().await,
            &rooms.peer_room,
            rooms.peer.id(),
        );
        let response = rooms
            .request_from_kernel_a(cancel(&rooms, AgentExecutionMode::Build))
            .await;
        assert!(
            matches!(
                response,
                RelayPeerResponse::PeerAgentAccepted {
                    duplicate: true,
                    ..
                }
            ),
            "{response:?}"
        );
        assert!(
            rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await,
            "an idle cancel's retry cannot cancel later work"
        );
    });
}
