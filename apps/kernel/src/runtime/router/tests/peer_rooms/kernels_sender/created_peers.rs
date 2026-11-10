//! Peers a room agent created in other rooms are deleted when that agent ends, on this kernel
//! and on another; agents it only messaged are not. A kernel that is down or restarts in between
//! only delays the delete.
use super::follow::offline::{link_stopped, relay_error, restarted};
use super::*;
use crate::local::{DestroyAgentRequest, EndSessionRequest, LocalDaemonRequest};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};

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
