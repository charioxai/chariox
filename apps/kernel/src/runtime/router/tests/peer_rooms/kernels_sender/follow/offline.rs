//! Peers across kernels when the link is not there: a kernel that is stopped, an answer that is
//! lost on the way back, a kernel that restarts or reconnects, a peer whose first turn ends before
//! its spawn is answered, and the card fence no peer request crosses.
use super::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::transport::relay_peer::{PeerAgentOutcome, PeerFollow};

/// What the relay answers when `kernel` is not connected, or dropped after it took the request.
pub(in super::super) fn relay_error(code: &str) -> DaemonError {
    DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: code.into(),
        message: format!("relay says {code}"),
        retryable: true,
    }
}

/// `kernel` is stopped: the relay answers every request with `target_not_connected`.
pub(in super::super) fn link_stopped(kernel: &Kernel) {
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            |_from: String, _owner: String, _request: RelayPeerRequest| {
                Box::pin(async { Err(relay_error("target_not_connected")) })
            },
        ),
    );
}

/// `kernel` is connected but never answers: every request to it waits for good.
fn link_never_answering(kernel: &Kernel) {
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            |_from: String, _owner: String, _request: RelayPeerRequest| {
                Box::pin(std::future::pending())
            },
        ),
    );
}

/// `kernel` acts on its first message, then drops off before the answer is back.
fn link_dropping_after_the_first_message(kernel: &Kernel) {
    let router = kernel.router.clone();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let dropped = dropped.clone();
                Box::pin(async move {
                    let message = matches!(request, RelayPeerRequest::PeerAgentMessage { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if message && !dropped.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return Err(relay_error("target_disconnected"));
                    }
                    answer
                })
            },
        ),
    );
}

/// `kernel` acts on its first cancel, then drops off before the answer is back.
fn link_dropping_after_the_first_cancel(kernel: &Kernel) {
    let router = kernel.router.clone();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let dropped = dropped.clone();
                Box::pin(async move {
                    let cancel = matches!(request, RelayPeerRequest::PeerAgentCancel { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if cancel && !dropped.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return Err(relay_error("target_disconnected"));
                    }
                    answer
                })
            },
        ),
    );
}

/// `kernel` acts on its first message, but its answer is not back before the caller stops waiting.
fn link_timing_out_after_the_first_message(kernel: &Kernel) {
    let router = kernel.router.clone();
    let late = Arc::new(std::sync::atomic::AtomicBool::new(false));
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let late = late.clone();
                Box::pin(async move {
                    let message = matches!(request, RelayPeerRequest::PeerAgentMessage { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if message && !late.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return Err(
                            crate::transport::relay_client::relay_peer_response_timed_out(Some(
                                std::time::Duration::from_secs(30),
                            )),
                        );
                    }
                    answer
                })
            },
        ),
    );
}

/// `kernel` boots again on the state it left, as after a restart, and answers peers again.
pub(in super::super) async fn restarted(kernel: Kernel) -> Kernel {
    let config = kernel.app.lock().await.config().clone();
    peer_agent_link_for_tests::unregister(&kernel.id);
    let Kernel {
        id,
        name,
        app,
        router,
        room,
        agent,
        token,
        prompt,
    } = kernel;
    drop(router);
    drop(app);
    let app = Arc::new(Mutex::new(
        crate::test_support::bootstrap_after_test_owner_exit(config).await,
    ));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
    let kernel = Kernel {
        id,
        name,
        app,
        router,
        room,
        agent,
        token,
        prompt,
    };
    link(&kernel);
    kernel
}

#[test]
fn a_stopped_kernel_is_unreachable_for_every_peer_request_by_reason_alone() {
    run_peer_test("peer-offline-stopped", || async {
        let (a, b) = two_kernels(
            "peer-offline-stopped",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let task = with_task(&a).await;
        link_stopped(&b);
        let message = a
            .send(serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "hello"}))
            .await
            .unwrap_err();
        let cancel = a
            .command(format!(
                "agent cancel worker --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        let follow = a
            .subscribe(
                &task,
                serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
            )
            .await
            .unwrap_err();
        let spawn = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        for refusal in [message, cancel, follow, spawn] {
            assert!(refusal.contains("kernel_unreachable"), "{refusal}");
            assert!(
                !refusal.contains("target_not_connected") && !refusal.contains("relay says"),
                "the reason, not the transport's words: {refusal}"
            );
        }
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_message_whose_answer_was_lost_is_uncertain_and_its_retry_is_one_delivery() {
    run_peer_test("peer-offline-uncertain", || async {
        let (a, b) = two_kernels(
            "peer-offline-uncertain",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_dropping_after_the_first_message(&b);
        let message = serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "parser.rs is mine"});
        let uncertain = a.send(message.clone()).await.unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        let key = uncertain
            .split("idempotency_key `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap_or_else(|| panic!("the refusal names the key to retry with: {uncertain}"))
            .to_owned();
        let mut retry = message;
        retry["idempotency_key"] = key.into();
        a.send(retry)
            .await
            .expect("the retry is answered by the kernel that took the first");
        assert_eq!(
            b.messages(b.agent.id()).await.len(),
            1,
            "an uncertain send and its retry are one delivery"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_cancel_whose_answer_was_lost_is_retried_without_cancelling_the_next_turn() {
    run_peer_test("peer-offline-cancel", || async {
        let (a, b) = two_kernels(
            "peer-offline-cancel",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_dropping_after_the_first_cancel(&b);
        let command = format!("agent cancel worker --room {} --kernel {}", b.room, b.name);
        let uncertain = a.command(command.clone()).await.unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        assert!(uncertain.contains("same call again"), "{uncertain}");
        assert!(
            !b.turn_running(b.agent.id()).await,
            "the first cancel acted"
        );
        b.end_turn(true).await;
        crate::test_support::admit_room_test_turn(&mut *b.app.lock().await, &b.room, b.agent.id());
        assert!(
            b.turn_running(b.agent.id()).await,
            "the peer's next turn runs"
        );
        let retry = a
            .command(command)
            .await
            .expect("the retry is answered with the first cancel's receipt");
        assert_eq!(retry["response"]["duplicate"], true, "{retry}");
        assert!(
            b.turn_running(b.agent.id()).await,
            "the retry leaves the peer's next turn running"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_steer_command_whose_answer_was_lost_is_one_delivery_when_made_again() {
    run_peer_test("peer-offline-steer", || async {
        let (a, b) = two_kernels(
            "peer-offline-steer",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_dropping_after_the_first_message(&b);
        let command = format!(
            "agent steer worker 'stop editing parser.rs' --room {} --kernel {}",
            b.room, b.name
        );
        let uncertain = a.command(command.clone()).await.unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        assert!(uncertain.contains("same call again"), "{uncertain}");
        let retry = a
            .command(command)
            .await
            .expect("the retry is answered by the kernel that took the first");
        assert_eq!(retry["response"]["status"], "duplicate", "{retry}");
        assert_eq!(
            b.messages(b.agent.id()).await.len(),
            1,
            "an uncertain steer and its retry are one delivery"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_message_that_timed_out_after_the_relay_took_it_is_uncertain_and_its_retry_is_one_delivery() {
    run_peer_test("peer-offline-timeout", || async {
        let (a, b) = two_kernels(
            "peer-offline-timeout",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_timing_out_after_the_first_message(&b);
        let message = serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "parser.rs is mine"});
        let uncertain = a.send(message.clone()).await.unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        assert!(
            !uncertain.contains("kernel_unreachable") && !uncertain.contains("timed out"),
            "the reason, not the transport's words: {uncertain}"
        );
        let key = uncertain
            .split("idempotency_key `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap_or_else(|| panic!("the refusal names the key to retry with: {uncertain}"))
            .to_owned();
        let mut retry = message;
        retry["idempotency_key"] = key.into();
        a.send(retry)
            .await
            .expect("the retry is answered by the kernel that took the first");
        assert_eq!(
            b.messages(b.agent.id()).await.len(),
            1,
            "a send that timed out and its retry are one delivery"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn an_outcome_owed_across_a_restart_of_the_followed_kernel_arrives_once() {
    run_peer_test("peer-offline-restart", || async {
        let (a, b, _task, _follow) = followed("peer-offline-restart").await;
        link_stopped(&a);
        b.end_turn(false).await;
        b.router.runtime_state.deliver_peer_agent_outcomes().await;
        assert_eq!(b.outbox().await.len(), 1, "owed while the follower is away");
        let b = restarted(b).await;
        assert_eq!(b.outbox().await.len(), 1, "the outbox outlives the restart");
        link(&a);
        // The restarted kernel's relay connection is a reconnect.
        b.router.runtime_state.peer_links_restored();
        b.router
            .runtime_state
            .peer_agent_outcome_deliveries_settled()
            .await;
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "turn_ended");
        assert!(b.outbox_drained().await);
        b.router.runtime_state.peer_links_restored();
        b.router
            .runtime_state
            .peer_agent_outcome_deliveries_settled()
            .await;
        assert_eq!(a.peer_events().await.len(), 1, "once");
        unlink(&[&a, &b]);
    });
}

#[test]
fn an_outcome_owed_to_a_kernel_that_was_away_is_delivered_when_the_relay_reconnects() {
    run_peer_test("peer-offline-reconnect", || async {
        let (a, b, _task, _follow) = followed("peer-offline-reconnect").await;
        link_stopped(&a);
        b.end_turn(false).await;
        b.router.runtime_state.deliver_peer_agent_outcomes().await;
        assert_eq!(b.outbox().await.len(), 1, "owed while the follower is away");
        link(&a);
        // The retry backoff has not run out: only the reconnect can bring the outcome now.
        b.router.runtime_state.peer_links_restored();
        b.router
            .runtime_state
            .peer_agent_outcome_deliveries_settled()
            .await;
        assert_eq!(a.peer_events().await.len(), 1, "delivered on reconnect");
        assert!(b.outbox().await.is_empty());
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_follower_that_never_answers_does_not_hold_the_lifecycle_sweep_of_unrelated_tasks() {
    run_peer_test("peer-offline-unanswered", || async {
        let (a, b, _task, _follow) = followed("peer-offline-unanswered").await;
        link_never_answering(&a);
        b.end_turn(false).await;
        // An unrelated task on B: a spawn of an agent that is gone is released by the sweep.
        let store = b.app.lock().await.durable_state_store();
        store
            .agent_lifecycle(Operation::RegisterObligation {
                owner: b.agent.owner_user_id().into(),
                room: b.room.clone(),
                agent: "bystander".into(),
                prompt: "bystander-turn".into(),
                run: None,
                id: "bystander-spawn".into(),
                kind: "delegate".into(),
                resource: Some("gone-child".into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        let sweep = b.router.runtime_state.sweep_agent_lifecycle_for_test();
        tokio::time::timeout(std::time::Duration::from_secs(10), sweep)
            .await
            .expect("the sweep does not wait on a follower that never answers")
            .unwrap();
        let bystander = store
            .agent_tasks(Some(&b.room), Some("bystander"))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            bystander.obligations[0].status, "cancelled",
            "{bystander:?}"
        );
        assert_eq!(
            b.outbox().await.len(),
            1,
            "still owed to the silent follower"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn follows_of_peers_on_other_kernels_are_lost_when_this_kernel_leaves_their_owner() {
    run_peer_test("peer-offline-unpaired", || async {
        let (a, b, _task, follow) = followed("peer-offline-unpaired").await;
        // As after `/kernels/unlink` or a Cloud answer that the link is gone.
        crate::runtime::cloud_relay_profile_store::clear_cloud_profile(&a.router.runtime_state)
            .await
            .unwrap();
        a.router
            .runtime_state
            .sweep_agent_lifecycle_for_test()
            .await
            .unwrap();
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "source_lost", "{:?}", events[0]);
        assert_eq!(events[0].source_id, source_of(&b));
        let held = a
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_registration(follow["registration_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(!held.live, "{held:?}");
        a.router
            .runtime_state
            .sweep_agent_lifecycle_for_test()
            .await
            .unwrap();
        assert_eq!(a.peer_events().await.len(), 1, "once");
        unlink(&[&a, &b]);
    });
}

/// Cloud's kernel directory as A reads it with its kernel credential, listing `kernel` with the
/// status `status` holds.
async fn kernel_directory(kernel: &Kernel, status: Arc<std::sync::Mutex<&'static str>>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let id = kernel.id.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                }
            }
            assert!(request.starts_with(b"GET /relay/targets?"));
            let status = *status.lock().unwrap();
            let body =
                serde_json::json!({"targets": [{"daemonId": id, "status": status}]}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    url
}

#[test]
fn follows_of_a_kernel_its_owner_unlinked_are_lost_and_of_a_stopped_one_kept() {
    run_peer_test("peer-offline-revoked", || async {
        let (a, b, _task, follow) = followed("peer-offline-revoked").await;
        let status = Arc::new(std::sync::Mutex::new("OFFLINE"));
        let api_url = kernel_directory(&b, status.clone()).await;
        let state = &a.router.runtime_state;
        crate::runtime::cloud_relay_profile_store::persist_cloud_profile(
            state,
            crate::config::PersistedCloudRelayProfile {
                api_url,
                user_id: OWNER.into(),
                account_id: "account-peer-offline".into(),
                realm_id: "realm-peer-offline".into(),
                kernel_credential: Some("synthetic-kernel-credential".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let now = crate::session::unix_epoch_ms();
        state.sweep_agent_lifecycle_at_for_test(now).await.unwrap();
        state.unpaired_peer_follow_settlements_settled().await;
        assert!(
            a.peer_events().await.is_empty(),
            "a stopped kernel keeps its followers"
        );
        // B was unlinked from the owner while A stays signed in.
        *status.lock().unwrap() = "REVOKED";
        state
            .sweep_agent_lifecycle_at_for_test(now + 60_000)
            .await
            .unwrap();
        state.unpaired_peer_follow_settlements_settled().await;
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "source_lost", "{:?}", events[0]);
        assert_eq!(events[0].source_id, source_of(&b));
        let held = a
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_registration(follow["registration_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(!held.live, "{held:?}");
        state
            .sweep_agent_lifecycle_at_for_test(now + 120_000)
            .await
            .unwrap();
        state.unpaired_peer_follow_settlements_settled().await;
        assert_eq!(a.peer_events().await.len(), 1, "once");
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_kernel_without_an_owner_keeps_its_follows_of_peers_in_its_own_rooms() {
    run_peer_test("peer-offline-local", || async {
        let a = kernel("peer-offline-local", AgentPermissionLevel::Yolo, false);
        let other = {
            let mut app = a.app.lock().await;
            let room = create_room(&mut app, "peer-offline-local-other", Some(OWNER));
            owned_agent(
                &mut app,
                &room,
                "neighbour",
                OWNER,
                AgentPermissionLevel::Yolo,
            );
            room
        };
        let task = with_task(&a).await;
        let follow = a
            .subscribe(
                &task,
                serde_json::json!({"source_id": "neighbour", "room": other}),
            )
            .await
            .expect("A follows a peer in another room of its own kernel");
        a.router
            .runtime_state
            .sweep_agent_lifecycle_for_test()
            .await
            .unwrap();
        assert!(a.peer_events().await.is_empty());
        let held = a
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_registration(follow["registration_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(held.live, "{held:?}");
    });
}

/// `kernel` creates the spawned peer, whose first turn then ends before the answer is back.
fn link_ending_the_spawned_turn_before_answering(kernel: &Kernel) {
    let router = kernel.router.clone();
    let app = kernel.app.clone();
    let room = kernel.room.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let (router, app, room) = (router.clone(), app.clone(), room.clone());
                Box::pin(async move {
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if let Ok(RelayPeerResponse::RoomPeerSpawned { agent, .. }) = &answer {
                        running_turn(&mut *app.lock().await, &room, &agent.agent_id);
                        router
                            .runtime_state
                            .end_agent_turn_for_tests(&room, &agent.agent_id, false)
                            .await?;
                        router.runtime_state.deliver_peer_agent_outcomes().await;
                    }
                    answer
                })
            },
        ),
    );
}

#[test]
fn a_spawned_peer_whose_first_turn_ends_before_the_spawn_is_answered_still_reaches_its_creator() {
    run_peer_test("peer-offline-spawn-window", || async {
        let (a, b) = two_kernels(
            "peer-offline-spawn-window",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_ending_the_spawned_turn_before_answering(&b);
        with_task(&a).await;
        let spawned = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .expect("a followed peer spawns on the other kernel");
        let peer = spawned["response"]["spawned"]["agent"]
            .as_str()
            .unwrap()
            .to_owned();
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, format!("{}/{}/{peer}", b.id, b.room));
        assert_eq!(events[0].payload["event"], "turn_ended");
        unlink(&[&a, &b]);
    });
}

/// `kernel` creates the spawned peer for the first spawn, then drops off before the answer is
/// back; it answers every later request. The peer it created is named in `created`.
fn link_losing_the_first_spawn_answer(kernel: &Kernel) -> Arc<std::sync::Mutex<Option<String>>> {
    let router = kernel.router.clone();
    let created = Arc::new(std::sync::Mutex::new(None));
    let seen = created.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let (router, seen) = (router.clone(), seen.clone());
                Box::pin(async move {
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if let Ok(RelayPeerResponse::RoomPeerSpawned { agent, .. }) = &answer {
                        let mut seen = seen.lock().unwrap();
                        if seen.is_none() {
                            *seen = Some(agent.agent_id.clone());
                            return Err(relay_error("target_disconnected"));
                        }
                    }
                    answer
                })
            },
        ),
    );
    created
}

/// A's live follows of peers, among them a spawn's follow whose peer is not known yet.
async fn live_peer_follows(kernel: &Kernel) -> usize {
    kernel
        .app
        .lock()
        .await
        .durable_state_store()
        .live_peer_registrations()
        .unwrap()
        .len()
}

#[test]
fn a_spawned_peer_whose_turn_ends_before_a_lost_spawn_answer_is_retried_reaches_its_creator_once() {
    run_peer_test("peer-offline-spawn-retry", || async {
        let (a, b) = two_kernels(
            "peer-offline-spawn-retry",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let created = link_losing_the_first_spawn_answer(&b);
        with_task(&a).await;
        let spawn = format!("agent spawn reviewer --room {} --kernel {}", b.room, b.name);
        let uncertain = a.command(spawn.clone()).await.unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        let peer = created.lock().unwrap().clone().expect("B created the peer");
        // The peer's first turn ends while A does not know it yet.
        running_turn(&mut *b.app.lock().await, &b.room, &peer);
        b.router
            .runtime_state
            .end_agent_turn_for_tests(&b.room, &peer, false)
            .await
            .unwrap();
        b.router.runtime_state.deliver_peer_agent_outcomes().await;
        let spawned = a
            .command(spawn)
            .await
            .expect("the retry reaches the peer the first spawn created");
        assert_eq!(spawned["response"]["spawned"]["agent"], peer.as_str());
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, format!("{}/{}/{peer}", b.id, b.room));
        assert_eq!(events[0].payload["event"], "turn_ended");
        b.router.runtime_state.deliver_peer_agent_outcomes().await;
        assert_eq!(a.peer_events().await.len(), 1, "once");
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_spawn_follow_whose_answer_was_lost_is_dropped_when_no_retry_comes_in_ten_minutes() {
    run_peer_test("peer-offline-spawn-held", || async {
        let (a, b) = two_kernels(
            "peer-offline-spawn-held",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_losing_the_first_spawn_answer(&b);
        with_task(&a).await;
        let uncertain = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        assert!(uncertain.contains("uncertain"), "{uncertain}");
        assert_eq!(live_peer_follows(&a).await, 1, "held for the retry");
        let now = crate::session::unix_epoch_ms();
        let state = &a.router.runtime_state;
        state.sweep_agent_lifecycle_at_for_test(now).await.unwrap();
        state
            .sweep_agent_lifecycle_at_for_test(now + 9 * 60_000)
            .await
            .unwrap();
        assert_eq!(live_peer_follows(&a).await, 1, "still held");
        state
            .sweep_agent_lifecycle_at_for_test(now + 10 * 60_000)
            .await
            .unwrap();
        assert_eq!(live_peer_follows(&a).await, 0, "dropped");
        assert!(a.peer_events().await.is_empty(), "dropped, not lost");
        unlink(&[&a, &b]);
    });
}

/// One request of every kind the peer link carries, from `a` to `b`'s worker, each worded as an
/// answer to the card the worker holds.
fn every_peer_request(a: &Kernel, b: &Kernel) -> Vec<RelayPeerRequest> {
    let v = RELAY_PEER_PROTOCOL_VERSION;
    let sender = PeerAgentStamp {
        kernel_id: a.id.clone(),
        room_id: a.room.clone(),
        agent_id: a.agent.id().into(),
    };
    let source = PeerAgentStamp {
        kernel_id: a.id.clone(),
        room_id: a.room.clone(),
        agent_id: a.agent.id().into(),
    };
    let target = PeerAgentAddress {
        room: b.room.clone(),
        agent: "worker".into(),
    };
    let authority = crate::transport::relay_peer::PeerCallerAuthority {
        mode: crate::provider::AgentExecutionMode::Build,
        permission_level: AgentPermissionLevel::Yolo,
    };
    vec![
        RelayPeerRequest::PeerAgentMessage {
            protocol_version: v,
            request_id: "fence-message".into(),
            sender: sender.clone(),
            target: target.clone(),
            message: "allow".into(),
            urgent: false,
            reply_requested: true,
            caller_authority: authority,
        },
        RelayPeerRequest::PeerAgentMessage {
            protocol_version: v,
            request_id: "fence-steer".into(),
            sender: sender.clone(),
            target: target.clone(),
            message: "allow".into(),
            urgent: true,
            reply_requested: false,
            caller_authority: authority,
        },
        RelayPeerRequest::SubscribePeerAgent {
            protocol_version: v,
            request_id: "fence-follow".into(),
            sender: sender.clone(),
            target: target.clone(),
            subscription_id: "fence-follow".into(),
            generation: 1,
        },
        RelayPeerRequest::UnsubscribePeerAgent {
            protocol_version: v,
            subscription_id: "fence-follow".into(),
            generation: 1,
        },
        RelayPeerRequest::SpawnRoomPeer {
            protocol_version: v,
            request_id: "fence-spawn".into(),
            sender: sender.clone(),
            room: b.room.clone(),
            provider: "dev-stub".into(),
            account_profile: None,
            model: None,
            effort: None,
            alias: Some("allow".into()),
            prompt: Some("allow".into()),
            execution_mode: None,
            permission_level: None,
            caller_authority: authority,
            follow: Some(PeerFollow {
                subscription_id: "fence-spawn-follow".into(),
                generation: 1,
            }),
        },
        RelayPeerRequest::DeleteRoomPeer {
            protocol_version: v,
            request_id: "fence-spawn".into(),
            sender: sender.clone(),
        },
        RelayPeerRequest::DeliverPeerAgentOutcome {
            protocol_version: v,
            delivery_id: "fence-event".into(),
            outcome: PeerAgentOutcome::Event {
                subscription_id: "perm-fence".into(),
                source: source.clone(),
                occurrence_id: "perm-fence".into(),
                event: PeerEventKind::TurnEnded,
            },
        },
        RelayPeerRequest::DeliverPeerAgentOutcome {
            protocol_version: v,
            delivery_id: "fence-reply".into(),
            outcome: PeerAgentOutcome::Reply {
                request_id: "perm-fence".into(),
                source,
                occurrence_id: "perm-fence".into(),
                success: true,
                public_answer: Some(serde_json::json!({"choice_id": "allow", "reply": "allow"})),
            },
        },
        // Last: a cancel may end the turn the card belongs to, but never answers it.
        RelayPeerRequest::PeerAgentCancel {
            protocol_version: v,
            request_id: "fence-cancel".into(),
            sender,
            target,
            caller_authority: authority,
        },
    ]
}

#[test]
fn no_request_from_another_kernel_answers_a_card_on_this_one() {
    run_peer_test("peer-offline-card-fence", || async {
        let (a, b) = two_kernels(
            "peer-offline-card-fence",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let requests = every_peer_request(&a, &b);
        // Every kind the link carries is here: a new kind fails this list until it is fenced.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../../../../fixtures/peer-agent-link-protocol.json"
        ))
        .unwrap();
        let mut pinned = fixture["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|request| request["kind"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut sent = requests
            .iter()
            .map(|request| {
                serde_json::to_value(request).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        pinned.sort();
        pinned.dedup();
        sent.sort();
        sent.dedup();
        assert_eq!(sent, pinned, "every peer request kind is fenced");

        // One card at a time is pending for an agent; the cancel, which may end its turn,
        // comes with the last one.
        let kinds = [
            ("perm-fence", RuntimeInteractionKind::Permission),
            ("choice-fence", RuntimeInteractionKind::Choice),
        ];
        for (round, (id, kind)) in kinds.into_iter().enumerate() {
            let last = round + 1 == kinds.len();
            let card = RuntimeInteraction::new(
                id,
                b.agent.id(),
                kind,
                RuntimeInteractionLevel::Warning,
                Some("Allow command?".into()),
                "Run `ls`?",
                vec![
                    RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                ],
                None,
                None,
                None,
            );
            let mut answer = b
                .router
                .runtime_state
                .create_runtime_interaction(&b.room, card)
                .await
                .unwrap();
            for request in every_peer_request(&a, &b) {
                let cancel = matches!(request, RelayPeerRequest::PeerAgentCancel { .. });
                if cancel && !last {
                    continue;
                }
                let sent = serde_json::to_value(&request).unwrap()["kind"].clone();
                let _ = b.router.relay_peer_agent(&a.id, OWNER, request).await;
                match answer.try_recv() {
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    // A cancel may end the turn its card belongs to; that is no answer.
                    Err(closed) if cancel => {
                        let _ = closed;
                    }
                    Err(closed) => panic!("{sent} closed card {id}: {closed:?}"),
                    Ok(resolved) => assert!(
                        cancel && resolved.choice_id.is_none() && resolved.reply.is_none(),
                        "{sent} answered card {id}: {resolved:?}"
                    ),
                }
            }
            if !last {
                b.router
                    .runtime_state
                    .resolve_runtime_interaction(&b.room, id, "deny", None)
                    .await
                    .unwrap();
            }
        }
        unlink(&[&a, &b]);
    });
}
