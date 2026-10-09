//! MP-08 / MP-10 / MP-11 A07: behavioral safety regressions, not live acceptance.
use super::*;
use crate::runtime::router::CommandRouter;
use crate::session::{HandoffChangeOp, RuntimeSession, DEFAULT_LOCAL_USER_ID};

fn fixture() -> (KernelRuntimeState, String) {
    fixture_with_owner(None, DEFAULT_LOCAL_USER_ID)
}
fn fixture_with_owner(
    cloud_owner: Option<&str>,
    session_owner: &str,
) -> (KernelRuntimeState, String) {
    let (router, id) = router_fixture_with_owner(cloud_owner, session_owner);
    (router.runtime_state().clone(), id)
}

fn router_fixture_with_owner(
    cloud_owner: Option<&str>,
    session_owner: &str,
) -> (CommandRouter, String) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.cloud_relay = cloud_owner.map(|owner| crate::config::PersistedCloudRelayProfile {
        user_id: owner.into(),
        ..Default::default()
    });
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let mut session = RuntimeSession::new(
        format!("am7-{:016x}", rand::random::<u64>()),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    session.set_owner_user_id(session_owner);
    let id = session.id().to_owned();
    app.sessions_mut().restore_session(session);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    (router, id)
}
fn handoff() -> RuntimeHandoff {
    RuntimeHandoff {
        kind: HandoffKind::Click,
        reason: HandoffReason::ModelRefusal,
        agent_id: "agent".into(),
        task_id: "task".into(),
        obligation_id: format!("obligation-{:016x}", rand::random::<u64>()),
        explanation: "Verify before saving the firewall".into(),
        target: HandoffTarget {
            tab_id: "tab".into(),
            generation: 1,
            document_id: "doc".into(),
            node_ref: "backend:7".into(),
            origin: "https://console.hetzner.cloud".into(),
            path: "/firewalls".into(),
            label: "Save".into(),
        },
        change: vec![
            HandoffChangeLine {
                op: HandoffChangeOp::Keep,
                text: "tcp 22 remains".into(),
            },
            HandoffChangeLine {
                op: HandoffChangeOp::Add,
                text: "tcp 443 from any".into(),
            },
        ],
        expires_at_ms: crate::session::unix_epoch_ms() + 900_000,
        save_to_vault_offered: false,
    }
}
async fn register(state: &KernelRuntimeState, room: &str, h: &RuntimeHandoff) -> String {
    state
        .register_handoff_interaction(room, DEFAULT_LOCAL_USER_ID, h.clone(), 900)
        .await
        .unwrap();
    RuntimeHandoff::interaction_id(&h.obligation_id)
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_s01_owner_only_bound_action_and_firewall_diff() {
    let (state, room) = fixture();
    let h = handoff();
    let id = register(&state, &room, &h).await;
    let session = state.owned.session_store.get_session(&room).unwrap();
    let projection = session
        .active_interactions()
        .iter()
        .find(|i| i.id() == id)
        .unwrap();
    assert_eq!(projection.handoff(), Some(&h));
    assert!(projection.message().contains("+ tcp 443 from any"));
    assert!(projection.message().contains("  tcp 22 remains"));
    assert!(state
        .owned
        .claim_handoff(&room, &id, "foreign-owner", |_| Ok(()))
        .is_err());
    assert!(state
        .answer_terminal_runtime_interaction(
            &room,
            &id,
            "done",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
            None,
            None,
            Some(crate::local::KernelConnectionClass::KernelAgent)
        )
        .await
        .is_err());
    let (claimed, _guard) = state
        .owned
        .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        .unwrap();
    assert_eq!(claimed.target, h.target);
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let safe = outcome(&id, HandoffStatus::Completed, "click", None);
    state.finish_handoff(&room, &h, &safe, DEFAULT_LOCAL_USER_ID);
    let events = state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "handoff.outcome", 1)
        .unwrap();
    assert_eq!(events[0].payload["outcome"]["status"], "completed");
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_s02_competing_terminals_late_replay_and_sweep() {
    let (state, room) = fixture();
    let h = handoff();
    let id = register(&state, &room, &h).await;
    let original = state.owned.session_store.get_session(&room).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let replies = std::thread::scope(|threads| {
        let a = threads.spawn(|| {
            barrier.wait();
            state
                .owned
                .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        });
        let b = threads.spawn(|| {
            barrier.wait();
            state
                .owned
                .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        });
        barrier.wait();
        vec![a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(
        replies.iter().filter(|reply| reply.is_ok()).count(),
        1,
        "competing terminals get exactly one claim"
    );
    let first = replies.into_iter().find_map(Result::ok).unwrap();
    assert!(state
        .owned
        .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        .is_err());
    assert_eq!(state.reconcile_handoff(&original, &id).await, None);
    assert!(
        state
            .owned
            .durable_state_store
            .load_subject_events_by_kind(&id, "handoff.outcome", 1)
            .unwrap()
            .is_empty(),
        "sweep cannot settle a still-running owner input"
    );
    drop(first);
    // A crash can restore a snapshot older than the write-ahead claim.
    state
        .owned
        .session_store
        .write()
        .restore_session(original.clone());
    assert!(state
        .owned
        .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        .is_err());
    state.reconcile_handoff(&original, &id).await;
    let events = state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "handoff.outcome", 1)
        .unwrap();
    assert_eq!(events[0].payload["outcome"]["status"], "uncertain");
    assert_eq!(
        events[0].payload["outcome"]["reason_code"],
        "restart_after_claim"
    );
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    assert_eq!(
        state
            .owned
            .durable_state_store
            .load_subject_events_by_kind(&id, "handoff.claimed", 200)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_s03_expiry_restart_and_owner_cancel_never_auto_answer() {
    let (state, room) = fixture();
    let mut h = handoff();
    h.expires_at_ms = 1;
    let id = register(&state, &room, &h).await;
    assert!(state
        .owned
        .claim_handoff(&room, &id, DEFAULT_LOCAL_USER_ID, |_| Ok(()))
        .is_err());
    state.owned.pending_interactions.write().remove(&id);
    let session = state.owned.session_store.get_session(&room).unwrap();
    state.reconcile_handoff(&session, &id).await;
    let events = state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "handoff.outcome", 1)
        .unwrap();
    assert_eq!(events[0].payload["outcome"]["status"], "expired");
    assert!(state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "handoff.claimed", 1)
        .unwrap()
        .is_empty());
    let h = handoff();
    let id = register(&state, &room, &h).await;
    state.owned.pending_interactions.write().remove(&id);
    let session = state.owned.session_store.get_session(&room).unwrap();
    state.reconcile_handoff(&session, &id).await;
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&room)
            .unwrap()
            .active_interactions()[0]
            .handoff()
            .unwrap()
            .expires_at_ms,
        h.expires_at_ms
    );
    assert!(state.withdraw_handoff(&room, &id));
    assert!(!state.withdraw_handoff(&room, &id));
    let events = state
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "handoff.outcome", 1)
        .unwrap();
    assert_eq!(events[0].payload["outcome"]["status"], "cancelled");
}

#[test]
fn mp08_mp10_mp11_a07_s04_values_never_become_command_metadata_or_cache() {
    let request = crate::local::LocalDaemonRequest::RespondToHandoff(RespondToHandoffRequest {
        session_id: "room".into(),
        interaction_id: "handoff-obligation".into(),
        action: HandoffResponseAction::EnterValue {
            value: crate::local::HandoffValue::new("private-fixture-991"),
            save_to_vault_key: None,
        },
    });
    let command =
        crate::runtime::command::KernelCommand::from_local_request("one", None, None, &request);
    assert!(!format!("{request:?}").contains("private-fixture-991"));
    assert!(!serde_json::to_string(&command)
        .unwrap()
        .contains("private-fixture-991"));
    assert!(!crate::runtime_transport::command_cache::request_is_cacheable(&request));
    assert_eq!(
        origin_and_path("https://example.com/login?code=private#value").unwrap(),
        ("https://example.com".into(), "/login".into())
    );
    assert!(origin_and_path("file:///etc/passwd").is_err());
}

#[test]
fn mp08_mp10_mp11_a07_s02_changed_or_disabled_field_is_refused() {
    use crate::runtime::browser_controller_snapshot::BrowserControllerDomNode;
    let mut n = BrowserControllerDomNode {
        node_ref: "backend:1".into(),
        parent_ref: None,
        document_index: 0,
        node_type: 1,
        node_name: "INPUT".into(),
        text: String::new(),
        attributes: BTreeMap::from([("type".into(), "password".into())]),
        bounds: None,
    };
    assert!(node_accepts(HandoffKind::Secret, &n));
    n.attributes.insert("readonly".into(), String::new());
    assert!(!node_accepts(HandoffKind::Secret, &n));
    n.attributes.remove("readonly");
    n.attributes.insert("type".into(), "hidden".into());
    assert!(!node_accepts(HandoffKind::Code, &n));
    n.attributes.insert("disabled".into(), String::new());
    assert!(!node_accepts(HandoffKind::Click, &n));
}

#[test]
fn mp08_mp10_mp11_a07_s02_query_only_navigation_changes_private_target_binding() {
    let first = document_url_binding("https://console.hetzner.cloud/firewalls?id=first");
    assert_ne!(
        first,
        document_url_binding("https://console.hetzner.cloud/firewalls?id=other")
    );
    assert_ne!(
        first,
        document_url_binding("https://console.hetzner.cloud/firewalls?id=first#other")
    );
    assert_eq!(first.len(), 64);
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_current_dispatch_fence_rejects_cancel_expiry_and_source_regrant() {
    use crate::durable_state::agent_lifecycle::Operation;
    for fault in ["cancel", "expiry", "regrant"] {
        let (state, room) = fixture();
        let mut h = handoff();
        h.task_id = "task-fence".into();
        let store = &state.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::Begin {
                owner: DEFAULT_LOCAL_USER_ID.into(),
                room: room.clone(),
                agent: h.agent_id.clone(),
                prompt: h.task_id.clone(),
                run: Some("run".into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::RegisterObligation {
                owner: DEFAULT_LOCAL_USER_ID.into(),
                room: room.clone(),
                agent: h.agent_id.clone(),
                prompt: h.task_id.clone(),
                run: Some("run".into()),
                id: h.obligation_id.clone(),
                kind: "hand_off".into(),
                resource: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        let host = &state.owned.kernel_browser_host;
        host.set_focus(DEFAULT_LOCAL_USER_ID, Some(&h.agent_id));
        host.load(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap();
        store.append_event("handoff.binding", Some(RuntimeHandoff::interaction_id(&h.obligation_id)),
            json!({"source_grant_identity":host.grant_identity(&host.admit(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap()).unwrap()})).unwrap();
        let source = host.admit(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap();
        let terminal = host.admit_terminal(DEFAULT_LOCAL_USER_ID, Default::default());
        if fault == "expiry" {
            h.expires_at_ms = crate::session::unix_epoch_ms() + 1_000;
        }
        let admission = state.handoff_action_admission(terminal, source, &h);
        assert!(host.check_admission(Some(&admission)).is_ok());
        match fault {
            "cancel" => {
                let task = store
                    .agent_tasks(Some(&room), Some(&h.agent_id))
                    .unwrap()
                    .remove(0);
                store
                    .agent_lifecycle(Operation::CancelTask {
                        task: h.task_id.clone(),
                        owner: DEFAULT_LOCAL_USER_ID.into(),
                        revision: task.revision,
                    })
                    .unwrap();
            }
            "expiry" => tokio::time::sleep(std::time::Duration::from_millis(1_100)).await,
            _ => {
                host.revoke_grants(DEFAULT_LOCAL_USER_ID, Some(&h.agent_id));
                host.set_focus(DEFAULT_LOCAL_USER_ID, Some(&h.agent_id));
                host.load(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap();
                assert!(
                    host.admit(DEFAULT_LOCAL_USER_ID, &h.agent_id).is_ok(),
                    "new authority is valid"
                );
            }
        }
        assert!(
            host.check_admission(Some(&admission)).is_err(),
            "old queued owner action must deny: {fault}"
        );
    }
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_local_and_hosted_owner_aliases_pending_source_regrant_is_fenced() {
    use crate::durable_state::agent_lifecycle::Operation;
    let (state, room) = fixture_with_owner(Some("cloud-owner"), DEFAULT_LOCAL_USER_ID);
    let mut h = handoff();
    h.task_id = "pending-source-task".into();
    let user = state.provider_account_authority_owner_user_id(DEFAULT_LOCAL_USER_ID);
    let store = &state.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: room.clone(),
            agent: h.agent_id.clone(),
            prompt: h.task_id.clone(),
            run: Some("run".into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room,
            agent: h.agent_id.clone(),
            prompt: h.task_id.clone(),
            run: Some("run".into()),
            id: h.obligation_id.clone(),
            kind: "hand_off".into(),
            resource: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let host = &state.owned.kernel_browser_host;
    host.set_focus(&user, Some(&h.agent_id));
    host.load(&user, &h.agent_id).unwrap();
    store.append_event("handoff.binding", Some(RuntimeHandoff::interaction_id(&h.obligation_id)),
        json!({"source_grant_identity":host.grant_identity(&host.admit(&user, &h.agent_id).unwrap()).unwrap()})).unwrap();
    host.revoke_grants(&user, Some(&h.agent_id));
    host.set_focus(&user, Some(&h.agent_id));
    host.load(&user, &h.agent_id).unwrap();
    let fresh = host.admit(&user, &h.agent_id).unwrap();
    let terminal = host.admit_terminal(&user, Default::default());
    let admission = state.handoff_action_admission(terminal, fresh, &h);
    assert!(
        host.check_admission(Some(&admission)).is_err(),
        "a fresh grant cannot authorize a hand-off requested before revocation"
    );
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_local_and_hosted_owner_aliases_models_cannot_bypass_pending_owner_step()
{
    let (state, room) = fixture();
    let h = handoff();
    register(&state, &room, &h).await;
    let host = &state.owned.kernel_browser_host;
    host.set_focus(DEFAULT_LOCAL_USER_ID, Some(&h.agent_id));
    host.load(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap();
    let admission = host.admit(DEFAULT_LOCAL_USER_ID, &h.agent_id).unwrap();
    let error = state
        .kernel_browser_operation_admitted(
            DEFAULT_LOCAL_USER_ID,
            Some(admission),
            "host.browser",
            json!({"op":"input","tab_id":h.target.tab_id,"generation":1,
            "document_id":"doc","input":{"kind":"click","x":1,"y":1}}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("owner_step_pending"), "{error}");
    assert_eq!(
        state
            .owned
            .session_store
            .get_session(&room)
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
}

#[test]
fn mp08_mp10_mp11_a07_publishes_approved_shared_request_contract() {
    let spec = handoff_spec();
    assert_eq!(spec.name, "chariox.handoff.request");
    assert_eq!(
        spec.input_schema["properties"]["kind"]["enum"],
        json!(["click", "code", "secret"])
    );
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert!(spec.description.contains("chariox.events.yield"));
}

#[tokio::test]
async fn mp08_mp10_mp11_a07_local_and_hosted_owner_aliases_answer_without_collaborator_authority() {
    for (owner, caller) in [
        (DEFAULT_LOCAL_USER_ID, "cloud-owner"),
        ("cloud-owner", DEFAULT_LOCAL_USER_ID),
    ] {
        let (state, room) = fixture_with_owner(Some("cloud-owner"), owner);
        let h = handoff();
        state
            .register_handoff_interaction(&room, owner, h.clone(), 900)
            .await
            .unwrap();
        let id = RuntimeHandoff::interaction_id(&h.obligation_id);
        assert!(state
            .owned
            .claim_handoff(&room, &id, "collaborator", |_| Ok(()))
            .is_err());
        assert!(
            state
                .owned
                .claim_handoff(&room, &id, caller, |_| Ok(()))
                .is_ok(),
            "same owner across terminals"
        );
    }
    let (state, room) = fixture_with_owner(Some("cloud-owner"), "collaborator");
    let h = handoff();
    state
        .register_handoff_interaction(&room, "collaborator", h.clone(), 900)
        .await
        .unwrap();
    let id = RuntimeHandoff::interaction_id(&h.obligation_id);
    for caller in ["cloud-owner", DEFAULT_LOCAL_USER_ID] {
        assert!(
            state
                .owned
                .claim_handoff(&room, &id, caller, |_| Ok(()))
                .is_err(),
            "home owner cannot answer a collaborator's hand-off"
        );
    }
    assert!(state
        .owned
        .claim_handoff(&room, &id, "collaborator", |_| Ok(()))
        .is_ok());
}

// MP-08 / MP-10 / MP-11: exercise the actual transport priority router, which
// rejected owner replies even though direct state tests passed.
#[tokio::test(flavor = "current_thread")]
async fn mp08_mp10_mp11_a07_owner_reply_reaches_interactive_router_once() {
    use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
    use crate::runtime::command::{KernelCommand, KernelCommandPriority};
    let (router, room) = router_fixture_with_owner(None, DEFAULT_LOCAL_USER_ID);
    let state = router.runtime_state();
    let h = handoff();
    let id = register(state, &room, &h).await;
    let request = LocalDaemonRequest::RespondToHandoff(RespondToHandoffRequest {
        session_id: room.clone(),
        interaction_id: id.clone(),
        action: HandoffResponseAction::Cancel,
    });
    let command = KernelCommand::from_local_request("owner-reply", None, None, &request);
    assert_eq!(command.priority, KernelCommandPriority::Interactive);
    assert!(crate::runtime::interactive_command_dispatcher::is_interactive_command(&request));
    let response = router
        .dispatch(command, request.clone())
        .await
        .expect("owner reply must reach its handler");
    assert!(
        matches!(response, LocalDaemonResponse::HandoffResolved { outcome } if outcome.status == HandoffStatus::Cancelled)
    );
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let replay = KernelCommand::from_local_request("owner-replay", None, None, &request);
    assert!(router.dispatch(replay, request).await.is_err());
}
