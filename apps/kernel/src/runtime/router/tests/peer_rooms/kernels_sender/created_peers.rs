//! Peers a room agent created in other rooms are deleted when that agent ends, on this kernel
//! and on another; agents it only messaged are not. A kernel that is down or restarts in between
//! only delays the delete.
use super::follow::offline::{link_stopped, relay_error, restarted};
use super::*;
use crate::local::{DestroyAgentRequest, EndSessionRequest, LocalDaemonRequest};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::transport::relay_peer::{
    PeerAgentRefusal, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

/// Deletes `agent` of `room` the way the owner's `agent delete` does.
async fn delete_agent(router: &CommandRouter, room: &str, agent: &str) {
    dispatch_as_owner(
        router,
        LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: room.into(),
            agent_id: agent.into(),
        }),
    )
    .await;
}

async fn dispatch_as_owner(router: &CommandRouter, request: LocalDaemonRequest) {
    let source = KernelCommandSource::DaemonBackground;
    let caller = KernelCaller {
        user_id: Some(OWNER.into()),
        ..KernelCaller::for_source(&source)
    };
    let command = KernelCommand::from_local_request_with_caller(
        "created-peer-test",
        source,
        caller,
        None,
        None,
        &request,
    );
    router
        .dispatch(command, request)
        .await
        .expect("the owner's request is accepted");
}

/// What `kernel` runs after a creator ended or the kernel started: finish the deletes.
async fn finish_deletes(router: &CommandRouter) {
    router.finish_created_peers().await;
}

async fn aliases(kernel: &Kernel, room: &str) -> Vec<String> {
    kernel
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(room)
        .into_iter()
        .filter_map(|agent| agent.alias().map(str::to_owned))
        .collect()
}

/// Polls until `alias` is gone from `room` of `kernel`; the delete runs in the background.
async fn gone_from(kernel: &Kernel, room: &str, alias: &str) -> bool {
    for _ in 0..100 {
        if !aliases(kernel, room).await.iter().any(|a| a == alias) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

async fn deleted_on_record(kernel: &Kernel, agent_id: &str) -> bool {
    kernel
        .app
        .lock()
        .await
        .durable_state_store()
        .agent_deletion_recorded(agent_id)
        .unwrap()
}

#[test]
fn a_peer_in_another_room_of_this_kernel_ends_with_its_creator() {
    run_peer_test("created-peer-local", || async {
        let a = kernel("created-peer-local-a", AgentPermissionLevel::Yolo, true);
        let other = {
            let mut app = a.app.lock().await;
            let other = create_room(&mut app, "other-room", Some(OWNER));
            owned_agent(
                &mut app,
                &other,
                "bystander",
                OWNER,
                AgentPermissionLevel::Yolo,
            );
            other
        };
        let spawned = a
            .command(format!("agent spawn reviewer --room {other} --no-follow"))
            .await
            .expect("a peer spawns in the other room");
        let peer = a
            .app
            .lock()
            .await
            .agents()
            .get_session_agents(&other)
            .into_iter()
            .find(|agent| agent.alias() == Some("reviewer"))
            .unwrap_or_else(|| panic!("the peer exists: {spawned}"));

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;

        assert!(
            gone_from(&a, &other, "reviewer").await,
            "the peer ends with its creator"
        );
        assert!(
            deleted_on_record(&a, peer.id()).await,
            "a normal, recorded delete"
        );
        assert!(
            aliases(&a, &other).await.contains(&"bystander".to_string()),
            "an agent the creator did not create stays"
        );
    });
}

#[test]
fn a_peer_on_another_kernel_ends_with_its_creator_and_other_agents_there_stay() {
    run_peer_test("created-peer-remote", || async {
        let (a, b) = two_kernels(
            "created-peer-remote",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let spawned = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {} --no-follow",
                b.room, b.name
            ))
            .await
            .expect("a peer spawns on the other kernel");
        let peer_id = spawned["response"]["spawned"]["agent"]
            .as_str()
            .unwrap()
            .to_string();
        // Messaging an agent there creates nothing, so it is not the creator's to delete.
        a.send(serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "hello"}))
            .await
            .expect("a message to an existing peer");

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;

        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the peer ends with its creator"
        );
        assert!(
            deleted_on_record(&b, &peer_id).await,
            "a normal, recorded delete on B"
        );
        assert!(
            aliases(&b, &b.room).await.contains(&"worker".to_string()),
            "the agent the creator only messaged stays"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_peer_whose_kernel_is_down_or_restarts_is_still_deleted_afterwards() {
    run_peer_test("created-peer-restart", || async {
        let (a, b) = two_kernels(
            "created-peer-restart",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        a.command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .expect("a peer spawns on the other kernel");

        // B is stopped when the creator ends: the delete cannot be made, and is not forgotten.
        link_stopped(&b);
        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(aliases(&b, &b.room).await.contains(&"reviewer".to_string()));

        // A restarts on the state it left, B is back: the next pass finishes the delete.
        let a = restarted(a).await;
        link(&b);
        finish_deletes(&a.router).await;
        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the peer is deleted after the restart"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_restart_of_the_creators_kernel_keeps_the_peers_of_a_creator_that_is_still_there() {
    run_peer_test("created-peer-live-restart", || async {
        let (a, b) = two_kernels(
            "created-peer-live-restart",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        a.command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .expect("a peer spawns on the other kernel");

        let a = restarted(a).await;
        finish_deletes(&a.router).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            aliases(&b, &b.room).await.contains(&"reviewer".to_string()),
            "the creator is still there, so is its peer"
        );
        unlink(&[&a, &b]);
    });
}

/// `kernel` acts on every request, but its answer to a spawn is lost on the way back.
fn link_dropping_after_the_spawn(kernel: &Kernel) {
    let router = kernel.router.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                Box::pin(async move {
                    let spawn = matches!(request, RelayPeerRequest::SpawnRoomPeer { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if spawn {
                        return Err(relay_error("target_disconnected"));
                    }
                    answer
                })
            },
        ),
    );
}

#[test]
fn a_peer_ends_when_the_room_of_its_creator_ends_and_a_lost_spawn_answer_leaves_it_deletable() {
    run_peer_test("created-peer-room-end", || async {
        let (a, b) = two_kernels(
            "created-peer-room-end",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        // B creates the peer, then drops off before the answer reaches the creator.
        link_dropping_after_the_spawn(&b);
        let lost = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {} --no-follow",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        assert!(lost.contains("uncertain"), "{lost}");
        assert!(aliases(&b, &b.room).await.contains(&"reviewer".to_string()));

        dispatch_as_owner(
            &a.router,
            LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: a.room.clone(),
            }),
        )
        .await;
        finish_deletes(&a.router).await;

        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the peer ends with the room of its creator, though the creator never learned its id"
        );
        unlink(&[&a, &b]);
    });
}

async fn set_room_alias(kernel: &Kernel, alias: &str) {
    let app = kernel.app.lock().await;
    let mut room = app.sessions().get_session(&kernel.room).unwrap();
    room.set_alias(Some(alias.into()));
    app.sessions_mut().restore_session(room);
}

async fn links(kernel: &Kernel) -> usize {
    kernel
        .app
        .lock()
        .await
        .durable_state_store()
        .created_peers()
        .unwrap()
        .len()
}

#[test]
fn a_lost_spawn_answer_keeps_its_link_when_the_retry_finds_the_kernel_offline() {
    run_peer_test("created-peer-offline-retry", || async {
        let (a, b) = two_kernels(
            "created-peer-offline-retry",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let spawn = format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        );
        link_dropping_after_the_spawn(&b);
        let lost = a.command(spawn.clone()).await.unwrap_err();
        assert!(lost.contains("uncertain"), "{lost}");

        // B is down when the same call is made again: that attempt learns nothing about the peer.
        link_stopped(&b);
        a.command(spawn).await.unwrap_err();
        assert_eq!(links(&a).await, 1, "the first attempt's link stays");

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;
        link(&b);
        finish_deletes(&a.router).await;
        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the peer the first attempt created ends with its creator"
        );
        unlink(&[&a, &b]);
    });
}

/// `kernel` refuses to delete the peer: its room is not one this owner can address.
fn link_refusing_the_delete(kernel: &Kernel) {
    let router = kernel.router.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                Box::pin(async move {
                    if matches!(request, RelayPeerRequest::DeleteRoomPeer { .. }) {
                        return Ok(RelayPeerResponse::PeerAgentRefused {
                            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                            refusal: PeerAgentRefusal::RoomUnavailable,
                        });
                    }
                    router.relay_peer_agent(&from, &owner, request).await
                })
            },
        ),
    );
}

#[test]
fn a_peer_in_a_room_the_owner_cannot_address_is_not_taken_for_deleted() {
    run_peer_test("created-peer-room-unavailable", || async {
        let (a, b) = two_kernels(
            "created-peer-room-unavailable",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        a.command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .expect("a peer spawns on the other kernel");
        link_refusing_the_delete(&b);

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;
        assert_eq!(
            links(&a).await,
            1,
            "the peer is still there: the link stays"
        );
        assert!(aliases(&b, &b.room).await.contains(&"reviewer".to_string()));

        link(&b);
        finish_deletes(&a.router).await;
        assert!(gone_from(&b, &b.room, "reviewer").await);
        assert_eq!(links(&a).await, 0);
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_peer_ends_with_its_creator_though_its_room_was_renamed() {
    run_peer_test("created-peer-renamed-room", || async {
        let (a, b) = two_kernels(
            "created-peer-renamed-room",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        set_room_alias(&b, "review-room").await;
        a.command(
            "agent spawn reviewer --room review-room --kernel ".to_string()
                + &b.name
                + " --no-follow",
        )
        .await
        .expect("a peer spawns by the room's alias");
        set_room_alias(&b, "renamed-room").await;

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;

        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the peer is found by its spawn, not by the alias the creator used"
        );
        assert_eq!(links(&a).await, 0, "and only then is the link released");
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_peer_whose_spawn_answer_was_lost_ends_with_its_creator_though_its_room_was_renamed() {
    run_peer_test("created-peer-renamed-room-lost", || async {
        let (a, b) = two_kernels(
            "created-peer-renamed-room-lost",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        set_room_alias(&b, "review-room").await;
        link_dropping_after_the_spawn(&b);
        a.command(
            "agent spawn reviewer --room review-room --kernel ".to_string()
                + &b.name
                + " --no-follow",
        )
        .await
        .unwrap_err();
        set_room_alias(&b, "renamed-room").await;

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;

        assert!(
            gone_from(&b, &b.room, "reviewer").await,
            "the creator never learned the room's id; the spawn's own key finds the peer"
        );
        unlink(&[&a, &b]);
    });
}

type HeldSpawn = Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>;

/// `kernel` takes a spawn request but does not act on it until `gate` opens, and the sender's
/// wait ends unanswered: a spawn still queued on `kernel` when its creator is deleted.
fn link_holding_the_spawn(kernel: &Kernel, gate: Arc<tokio::sync::Notify>) -> HeldSpawn {
    let held: HeldSpawn = Default::default();
    let router = kernel.router.clone();
    let slot = held.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let gate = gate.clone();
                let slot = slot.clone();
                Box::pin(async move {
                    if matches!(request, RelayPeerRequest::SpawnRoomPeer { .. }) {
                        *slot.lock().unwrap() = Some(tokio::spawn(async move {
                            gate.notified().await;
                            let _ = router.relay_peer_agent(&from, &owner, request).await;
                        }));
                        return Err(relay_error("target_disconnected"));
                    }
                    router.relay_peer_agent(&from, &owner, request).await
                })
            },
        ),
    );
    held
}

#[test]
fn a_delete_that_reaches_the_kernel_before_the_queued_spawn_leaves_no_orphan() {
    run_peer_test("created-peer-delete-before-spawn", || async {
        let (a, b) = two_kernels(
            "created-peer-delete-before-spawn",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let gate = Arc::new(tokio::sync::Notify::new());
        let held = link_holding_the_spawn(&b, gate.clone());
        a.command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .unwrap_err();

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        finish_deletes(&a.router).await;

        // The queued spawn commits after the delete was answered.
        gate.notify_one();
        let spawn = held.lock().unwrap().take().expect("B took the spawn");
        spawn.await.unwrap();
        assert!(
            !aliases(&b, &b.room).await.contains(&"reviewer".to_string()),
            "a spawn of a creator that already ended creates nothing"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_delete_waits_for_the_spawn_that_is_creating_the_peer() {
    run_peer_test("created-peer-delete-waits", || async {
        let (a, b) = two_kernels(
            "created-peer-delete-waits",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let gate = Arc::new(tokio::sync::Notify::new());
        let held = link_holding_the_spawn(&b, gate.clone());
        a.command(format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        ))
        .await
        .unwrap_err();
        let request_id = a
            .app
            .lock()
            .await
            .durable_state_store()
            .created_peers()
            .unwrap()[0]
            .peer
            .clone();
        // The spawn is acting on B: the claim is its.
        let claim = b
            .router
            .runtime_state
            .claim_peer_spawn_when_free(&a.id, &request_id)
            .await;

        delete_agent(&a.router, &a.room, a.agent.id()).await;
        let router = a.router.clone();
        let pass = tokio::spawn(async move { router.finish_created_peers().await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!pass.is_finished(), "the delete waits for the spawn to end");
        assert_eq!(links(&a).await, 1);

        drop(claim);
        pass.await.unwrap();
        assert_eq!(links(&a).await, 0, "then it is done");
        gate.notify_one();
        let spawn = held.lock().unwrap().take().expect("B took the spawn");
        spawn.await.unwrap();
        assert!(!aliases(&b, &b.room).await.contains(&"reviewer".to_string()));
        unlink(&[&a, &b]);
    });
}
