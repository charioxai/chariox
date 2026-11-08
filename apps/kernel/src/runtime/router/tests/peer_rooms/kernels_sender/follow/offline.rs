//! Peers across kernels when the link is not there: a kernel that is stopped, an answer that is
//! lost on the way back, a kernel that restarts or reconnects, a peer whose first turn ends before
//! its spawn is answered, and the card fence no peer request crosses.
use super::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::transport::relay_peer::{PeerAgentOutcome, PeerFollow};

/// What the relay answers when `kernel` is not connected, or dropped after it took the request.
fn relay_error(code: &str) -> DaemonError {
    DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: code.into(),
        message: format!("relay says {code}"),
        retryable: true,
    }
}

/// `kernel` is stopped: the relay answers every request with `target_not_connected`.
fn link_stopped(kernel: &Kernel) {
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
async fn restarted(kernel: Kernel) -> Kernel {
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
        b.router
            .runtime_state
            .deliver_peer_agent_outcomes()
            .await
            .unwrap();
        assert_eq!(b.outbox().await.len(), 1, "owed while the follower is away");
        let b = restarted(b).await;
        assert_eq!(b.outbox().await.len(), 1, "the outbox outlives the restart");
        link(&a);
        // The restarted kernel's relay connection is a reconnect.
        b.router.runtime_state.peer_links_restored().await;
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "turn_ended");
        assert!(b.outbox_drained().await);
        b.router.runtime_state.peer_links_restored().await;
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
        b.router
            .runtime_state
            .deliver_peer_agent_outcomes()
            .await
            .unwrap();
        assert_eq!(b.outbox().await.len(), 1, "owed while the follower is away");
        link(&a);
        // The retry backoff has not run out: only the reconnect can bring the outcome now.
        b.router.runtime_state.peer_links_restored().await;
        assert_eq!(a.peer_events().await.len(), 1, "delivered on reconnect");
        assert!(b.outbox().await.is_empty());
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
                        router.runtime_state.deliver_peer_agent_outcomes().await?;
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
