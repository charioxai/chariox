// MP-08 / MP-10 / MP-11: transport-envelope regression; supplementary, not live acceptance.
use super::*;

async fn assert_home_handoff_envelope(active_run: bool, current_lease_output: bool, protect: bool) {
    assert_home_handoff_envelope_with_recovery(active_run, current_lease_output, protect, false)
        .await;
}

async fn assert_home_handoff_envelope_with_recovery(
    active_run: bool,
    current_lease_output: bool,
    protect: bool,
    recovered: bool,
) {
    assert_home_handoff_scenario(
        active_run,
        current_lease_output,
        protect,
        recovered,
        None,
        false,
    )
    .await;
}

async fn assert_home_handoff_scenario(
    active_run: bool,
    current_lease_output: bool,
    protect: bool,
    recovered: bool,
    workflow_flush: Option<bool>,
    protection_unavailable: bool,
) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
    config.relay_token = Some("recreated-handoff-test-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();

    let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "recreated-handoff-workspace",
            "recreated-handoff-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "recreated-handoff-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-recreated-handoff".to_string(),
                execution_lease_id: "lease-recreated-handoff".to_string(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: active_run.then(|| "worker-current-run".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("home agent should bind to the fake worker");

    // The worker restarted: its new lease has no native conversation, while
    // the home still owns the earlier worker's user and assistant records.
    let prior_run = crate::provider::projected_leased_provider_run_id(
        if current_lease_output {
            CANCELLATION_GUARD_LEASED_AGENT_ID
        } else {
            "old-leased-agent"
        },
        "provider-run-1",
    );
    let history = app.operational_history_store();
    for (sequence, entry) in [
        (
            1,
            crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                attachment.id(),
                agent.id(),
                "accepted goal: audit continuity; private-room-fixture",
            ),
        ),
        (
            2,
            crate::history::SessionHistoryEntry::provider_output(
                session.id(),
                &prior_run,
                Some(agent.id()),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                "assistant proposal: preserve named decisions",
            ),
        ),
    ] {
        history
            .append(&crate::history::HistoryEvent::transcript(
                sequence,
                &entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(agent.provider().to_string()),
                    provider_run_id: Some(prior_run.clone()),
                    prompt_id: Some("prior-home-prompt".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .unwrap();
    }

    if !active_run && !current_lease_output {
        let current = app.agents.get_agent(agent.id()).unwrap();
        app.operational_history_store()
            .mark_remote_lease_context_pending(
                session.id(),
                agent.id(),
                current.remote_execution().unwrap(),
            )
            .unwrap();
    }
    let workflow_ids = workflow_flush.map(|flush| {
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("replay-workflow".into()))
            .unwrap();
        app.sessions_mut()
            .set_workflow_flush_agent_context_before_run(session.id(), workflow.id(), flush)
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("continue audit".into()),
            )
            .unwrap();
        (run.id().to_string(), run.node_runs()[0].id().to_string())
    });
    let app = Arc::new(Mutex::new(app));
    let mut runtime = owned_runtime_state(&app).await;
    if recovered {
        runtime
            .owned
            .room_secret_observations
            .reset_environment(session.id())
            .unwrap();
    }
    if protect {
        runtime
            .owned
            .room_secret_observations
            .register(session.id(), "private-room-fixture")
            .unwrap();
    }
    let unavailable_root = std::env::temp_dir().join(format!(
        "ctxlease-unavailable-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    if protection_unavailable {
        std::fs::write(&unavailable_root, "not a directory").unwrap();
        runtime.owned.room_secret_observations =
            super::super::room_secret_observation::RoomSecretObservations::new(
                unavailable_root.clone(),
                Default::default(),
            );
    }
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let mut submission = runtime
        .owned
        .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: {
                let prompt = PromptQueueItem::new(
                    "pending:recreated-handoff",
                    &attachment_id,
                    &agent_id,
                    "hold the worker response while manifest sync runs",
                    PromptStatus::Queued,
                );
                if let Some((run, node)) = &workflow_ids {
                    prompt.with_workflow_context(run, node)
                } else {
                    prompt
                }
            },
            force_queue: false,
            refresh_projection: true,
        })
        .expect("remote prompt should be admitted")
        .expect("remote prompt should produce a dispatch");
    let mut dispatch = submission
        .remote_dispatch
        .take()
        .expect("accepted remote prompt should carry a dispatch");
    if let Some((run, node)) = &workflow_ids {
        dispatch.workflow_context = Some(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: "home-kernel".into(),
            home_session_id: session_id.clone(),
            home_agent_id: agent_id.clone(),
            workflow_run_id: run.clone(),
            workflow_node_run_id: node.clone(),
            delivery_token: "test-workflow-delivery".into(),
        });
    }
    if protect {
        // Cached background was prepared before the Room became protected.
        dispatch.hidden_system_context = "cached setting: private-room-fixture".to_string();
    }
    let required_context = if workflow_ids.is_some() {
        let (_, hidden) = crate::prompt_transcript::split_workflow_prompt_for_hidden_context(
            "handoff payload\n<node-level-prompt>Read only; never edit source.</node-level-prompt>\n<workflow-runtime-instructions>Delivery: test-workflow-delivery; acknowledge and submit output.</workflow-runtime-instructions>\n<system-node-level-prompt>Preserve node constraints.</system-node-level-prompt>".to_string(),
        );
        hidden
    } else {
        "Failed request: retry the audit. Steer: preserve native sessions.".to_string()
    };
    if protection_unavailable {
        dispatch.hidden_system_context = required_context.clone();
    }
    let settlement_dispatch = dispatch.clone();
    let submitted_prompt = PromptQueueItem::new(
        format!("worker-{}", dispatch.prompt_id),
        "worker-attachment",
        &dispatch.leased_agent_id,
        dispatch.prompt.clone(),
        PromptStatus::Running,
    );

    let relay_state = Arc::clone(&runtime.owned.relay_state);
    let (outgoing_tx, mut peer_requests, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, CANCELLATION_GUARD_RELAY_URL);
        relay.remember_peer_public_key(
            CANCELLATION_GUARD_WORKER_ID,
            worker_config.relay_public_key.clone(),
        );
    }

    let submit_runtime = runtime.clone();
    let submit_prompt = dispatch.prompt.clone();
    let submit_task = tokio::spawn(async move {
        submit_remote_prompt_to_worker_with_binding_refresh(
            &submit_runtime,
            &mut dispatch,
            submit_prompt,
            Vec::new(),
        )
        .await
    });

    let (submit_request_id, request) =
        receive_fake_worker_request(&mut peer_requests, &worker_config.relay_private_key).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        prompt,
        hidden_system_context,
        ..
    } = request
    else {
        panic!("expected the worker prompt envelope")
    };
    assert_eq!(prompt, "hold the worker response while manifest sync runs");
    if protection_unavailable {
        assert_eq!(hidden_system_context, required_context, "current turn instructions must reach the worker unchanged when optional replay is withheld");
    }
    if protect {
        assert!(
            !hidden_system_context.contains("private-room-fixture"),
            "cached context must obey current Room protection even without history replay"
        );
    }
    if active_run || current_lease_output || workflow_flush == Some(true) || protection_unavailable
    {
        assert!(
            !hidden_system_context.contains("accepted goal: audit continuity"),
            "same-lease native continuity must skip home replay"
        );
    } else {
        assert!(
            hidden_system_context.contains("accepted goal: audit continuity"),
            "a recreated worker must receive the home's earlier user history"
        );
        assert_eq!(
            hidden_system_context.contains("assistant proposal: preserve named decisions"),
            !protect && !recovered,
            "current Room policy withholds old provider observations when protection begins",
        );
        assert!(hidden_system_context.len() <= 24_000);
        assert_eq!(
            hidden_system_context.contains("private-room-fixture"),
            !protect && !recovered,
            "current Room policy must protect the actual outgoing envelope"
        );
    }

    let submit_response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: "provider-run-recreated-handoff".to_string(),
        outcome: crate::session::PromptSubmissionOutcome::Started {
            prompt: submitted_prompt,
        },
    };
    let encrypted_submit_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&submit_response).expect("submit response should encode"),
    )
    .expect("fake worker should encrypt the submit response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        submit_request_id,
        CANCELLATION_GUARD_WORKER_ID.to_string(),
        encrypted_submit_response,
    )
    .await;
    assert_eq!(
        submit_task
            .await
            .expect("prompt submission task should not panic")
            .expect("prompt submission should succeed"),
        "provider-run-recreated-handoff"
    );
    let accepted = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(
            &settlement_dispatch,
            Some("provider-run-recreated-handoff"),
        )
        .unwrap();
    let pending = runtime
        .owned
        .operational_history_store
        .remote_lease_context_pending(&session_id, &agent_id, accepted.remote_execution().unwrap())
        .unwrap();
    assert_eq!(
        pending,
        workflow_flush == Some(true) || protection_unavailable,
        "only replay-bearing acknowledgements consume the marker"
    );
    if workflow_flush == Some(true) {
        let next = runtime
            .owned
            .remote_prompt_hidden_context(
                &accepted,
                CANCELLATION_GUARD_LEASED_AGENT_ID,
                "ordinary-next",
                "",
                false,
                false,
            )
            .unwrap();
        assert!(
            next.contains("accepted goal: audit continuity"),
            "fresh workflow acceptance must leave replay for the next ordinary prompt"
        );
    }
    if protection_unavailable {
        std::fs::remove_file(unavailable_root).unwrap();
    }
}

#[tokio::test]
async fn recreated_lease_replays_home_history_in_hidden_context() {
    assert_home_handoff_envelope(false, false, false).await;
}

#[tokio::test]
async fn accepted_worker_run_does_not_replay_home_history() {
    assert_home_handoff_envelope(true, false, false).await;
}

#[tokio::test]
async fn current_lease_history_keeps_native_continuity() {
    assert_home_handoff_envelope(false, true, false).await;
}

#[tokio::test]
async fn recreated_lease_handoff_uses_current_room_protection() {
    assert_home_handoff_envelope(false, false, true).await;
}

#[tokio::test]
async fn active_lease_scrubs_cached_context_without_replaying_history() {
    assert_home_handoff_envelope(true, false, true).await;
}

#[tokio::test]
async fn current_lease_scrubs_cached_context_without_replaying_history() {
    assert_home_handoff_envelope(false, true, true).await;
}

#[tokio::test]
async fn recovered_room_scrubs_cached_context_before_remote_transport() {
    assert_home_handoff_envelope_with_recovery(true, false, true, true).await;
}

// MP-08 / MP-10 / MP-11: home history keeps the current replica namespace.
#[tokio::test]
async fn review976_completed_projection_skips_replay_until_lease_identity_changes() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "projection-workspace",
            "projection-worktree",
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.into(),
                worker_machine_id: "projection-machine".into(),
                execution_lease_id: "projection-lease".into(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        "worker-session",
        "dev-stub",
        "codex",
        "default",
        "model",
    )
    .with_agent_id("worker-agent")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "raw-worker-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "projection-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.append_history_entry(
        session.id(),
        crate::history::SessionHistoryEntry::user_prompt(
            session.id(),
            "home-attachment",
            agent.id(),
            "earlier accepted task",
        ),
    );
    let event = crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        provider_run_id: run.id().into(),
        provider_run: Some(run),
        prompts: Vec::new(),
        output_chunks: vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderOutput,
            merge_key: None,
            bytes: b"completed assistant answer".to_vec(),
        }],
        notices: Vec::new(),
        completions: Vec::new(),
    };
    assert!(
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority(
                    CANCELLATION_GUARD_WORKER_ID
                ),
                event,
            )
            .unwrap()
            .accepted
    );
    app.agents
        .set_remote_execution_active_worker_provider_run_id(agent.id(), None)
        .unwrap();
    let events = app
        .operational_history_store()
        .load_session_events(session.id(), Some(agent.id()))
        .unwrap();
    let home_run_id = format!("leased:{CANCELLATION_GUARD_LEASED_AGENT_ID}:raw-worker-run");
    assert!(events.iter().any(|event| event.kind
        == crate::history::HistoryEventKind::ProviderOutput
        && event.provider_run_id.as_deref() == Some(home_run_id.as_str())));
    assert!(events
        .iter()
        .all(|event| event.provider_run_id.as_deref() != Some("raw-worker-run")));
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let current = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
    let context = runtime
        .owned
        .remote_prompt_hidden_context(
            &current,
            CANCELLATION_GUARD_LEASED_AGENT_ID,
            "next",
            "existing context",
            false,
            true,
        )
        .unwrap();
    assert_eq!(
        context, "existing context",
        "completed current-lease turns must not replay history"
    );
    let mut replacement = current.remote_execution().unwrap().clone();
    replacement.execution_lease_id = "recreated-execution-lease".into();
    replacement.leased_agent_id = "recreated-leased-agent".into();
    let rebound = runtime
        .owned
        .agent_store
        .bind_remote_execution(agent.id(), replacement)
        .unwrap();
    runtime
        .owned
        .operational_history_store
        .mark_remote_lease_context_pending(
            session.id(),
            agent.id(),
            rebound.remote_execution().unwrap(),
        )
        .unwrap();
    let reopened = crate::history::OperationalHistoryStore::open(
        runtime.owned.operational_history_store.path().to_path_buf(),
    )
    .unwrap();
    assert!(reopened
        .remote_lease_context_pending(
            session.id(),
            agent.id(),
            rebound.remote_execution().unwrap()
        )
        .unwrap());
    let context = runtime
        .owned
        .remote_prompt_hidden_context(
            &rebound,
            "recreated-leased-agent",
            "next",
            "existing context",
            false,
            true,
        )
        .unwrap();
    assert!(context.contains("earlier accepted task"));
    assert!(context.contains("completed assistant answer"));
    // A newly launched native process on the recreated lease is not proof that
    // its conversation already received the home's earlier turns.
    let mut running = rebound.clone();
    running.set_remote_execution_active_worker_provider_run_id(Some("new-native-run".into()));
    assert_eq!(
        runtime
            .owned
            .remote_prompt_hidden_context(
                &running,
                "recreated-leased-agent",
                "next",
                "existing context",
                false,
                true
            )
            .unwrap(),
        context
    );
    assert_eq!(
        runtime
            .owned
            .remote_prompt_hidden_context(
                &rebound,
                "recreated-leased-agent",
                "next",
                "existing context",
                false,
                true
            )
            .unwrap(),
        context
    );
    reopened
        .clear_remote_lease_context_pending(
            session.id(),
            agent.id(),
            CANCELLATION_GUARD_WORKER_ID,
            "old-leased-agent",
        )
        .unwrap();
    assert!(reopened
        .remote_lease_context_pending(
            session.id(),
            agent.id(),
            rebound.remote_execution().unwrap()
        )
        .unwrap());
    reopened
        .clear_remote_lease_context_pending(
            session.id(),
            agent.id(),
            CANCELLATION_GUARD_WORKER_ID,
            "recreated-leased-agent",
        )
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .remote_prompt_hidden_context(
                &rebound,
                "recreated-leased-agent",
                "next",
                "existing context",
                false,
                true
            )
            .unwrap(),
        "existing context"
    );
}

#[tokio::test]
async fn review976_history_read_failure_keeps_scrubbed_existing_context() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "history-workspace",
            "history-worktree",
        ))
        .unwrap();
    let agent = app
        .agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.into(),
                worker_machine_id: "history-machine".into(),
                execution_lease_id: "history-lease".into(),
                leased_agent_id: "new-lease".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let history = app.operational_history_store();
    history
        .mark_remote_lease_context_pending(
            session.id(),
            agent.id(),
            agent.remote_execution().unwrap(),
        )
        .unwrap();
    history
        .append(&crate::history::HistoryEvent::transcript(
            1,
            &crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                "attachment",
                agent.id(),
                "previous task",
            ),
            crate::history::HistoryEventTurnContext::default(),
        ))
        .unwrap();
    let connection = rusqlite::Connection::open(history.path()).unwrap();
    connection
        .execute(
            "UPDATE history_events SET event_json = 'broken-json' WHERE agent_id = ?1",
            [agent.id()],
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .room_secret_observations
        .register(session.id(), "history-canary")
        .unwrap();
    let context = runtime
        .owned
        .remote_prompt_hidden_context(
            &agent,
            "new-lease",
            "next",
            "required context history-canary",
            false,
            false,
        )
        .expect("optional replay failure must not block a turn");
    assert!(context.contains("required context"));
    assert!(!context.contains("history-canary"));
}

// MP-08 / MP-10 / MP-11: actual workflow policy, outgoing envelope, submission and durable ACK.
#[tokio::test]
async fn review976r2_continuing_workflow_receives_pending_replay() {
    assert_home_handoff_scenario(false, false, false, false, Some(false), false).await;
}
#[tokio::test]
async fn review976r2_fresh_workflow_ack_preserves_pending_replay() {
    assert_home_handoff_scenario(false, false, false, false, Some(true), false).await;
}
#[tokio::test]
async fn review976r3_unavailable_room_preserves_current_context_without_replay() {
    assert_home_handoff_scenario(false, false, false, false, None, true).await;
}

// MP-08 / MP-10 / MP-11: workflow instructions survive the actual worker envelope.
#[tokio::test]
async fn review976r3_unavailable_room_preserves_workflow_instructions_without_replay() {
    assert_home_handoff_scenario(false, false, false, false, Some(false), true).await;
}

#[tokio::test]
async fn a_profile_change_on_a_recreated_lease_replays_home_history_again() {
    // The worker builds a provider switch's handoff from its own history,
    // which on a recreated lease starts at the refresh. The home replays.
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "rearm-workspace",
            "rearm-worktree",
        ))
        .unwrap();
    let binding = crate::agent::RemoteAgentBinding {
        worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.into(),
        worker_machine_id: "rearm-machine".into(),
        execution_lease_id: "recreated-lease".into(),
        leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.into(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    app.agents
        .bind_remote_execution(agent.id(), binding.clone())
        .unwrap();
    app.append_history_entry(
        session.id(),
        crate::history::SessionHistoryEntry::user_prompt(
            session.id(),
            "home-attachment",
            agent.id(),
            "earlier accepted task",
        ),
    );
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let history = &runtime.owned.operational_history_store;
    history
        .mark_remote_lease_context_pending(session.id(), agent.id(), &binding)
        .unwrap();
    let context = |prompt_id: &str| {
        let current = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
        runtime
            .owned
            .remote_prompt_hidden_context(
                &current,
                CANCELLATION_GUARD_LEASED_AGENT_ID,
                prompt_id,
                "existing context",
                false,
                false,
            )
            .unwrap()
    };
    assert!(context("first").contains("earlier accepted task"));
    history
        .clear_remote_lease_context_if_replayed(
            session.id(),
            agent.id(),
            CANCELLATION_GUARD_WORKER_ID,
            CANCELLATION_GUARD_LEASED_AGENT_ID,
            "first",
        )
        .unwrap();
    assert_eq!(context("second"), "existing context");
    runtime
        .owned
        .commit_remote_agent_profile_update(
            session.id(),
            agent.id(),
            agent.provider().to_string(),
            agent.provider_account_profile().to_string(),
            Some("switched-model".into()),
            None,
        )
        .unwrap();
    assert!(
        context("third").contains("earlier accepted task"),
        "the switched provider session must receive the home conversation"
    );
}
