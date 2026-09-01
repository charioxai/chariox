use super::*;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

#[test]
fn bootstrap_restores_created_session_and_agents_from_durable_state() {
    let config = DaemonConfig::for_tests();
    let (session_id, default_agent_id, reviewer_agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let reviewer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("agent should spawn");
        (
            session.id().to_string(),
            default_agent.id().to_string(),
            reviewer.id().to_string(),
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_session = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert_eq!(restored_session.id(), session_id);
    assert_eq!(
        app.agents
            .get_agent(&default_agent_id)
            .expect("default agent should restore")
            .session_id(),
        session_id
    );
    assert_eq!(
        app.agents
            .get_agent(&reviewer_agent_id)
            .expect("spawned agent should restore")
            .session_id(),
        session_id
    );
}

#[test]
fn bootstrap_ignores_foreign_session_update_before_decoding_newer_schema() {
    let config = DaemonConfig::for_tests();
    {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut foreign_session = serde_json::to_value(&session).expect("session should serialize");
        foreign_session["host_daemon_id"] = serde_json::json!("newer-foreign-kernel");
        foreign_session["workflow_queued_prompts"] = serde_json::json!([{
            "id": "foreign-event-prompt",
            "queue_id": "foreign-workflow:default",
            "workflow_id": "foreign-workflow",
            "endpoint_id": "foreign-endpoint",
            "prompt": "foreign event prompt",
            "source": "event",
            "status": "queued",
            "created_at_ms": 1,
            "updated_at_ms": 1
        }]);
        app.durable_state_store()
            .append_event(
                "session.updated",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": foreign_session,
                    "reason": "foreign_kernel_newer_schema",
                }),
            )
            .expect("foreign session event should persist");
    }

    DaemonApp::bootstrap(config)
        .expect("foreign session state should be ignored before typed decoding");
}

#[test]
fn bootstrap_restores_unexpired_workflow_publication_tunnel_intent() {
    let config = DaemonConfig::for_tests();
    let tunnel_id = "publication-durable-restart";
    let local_url = "http://127.0.0.1:43100/publications/runtime/public-api";
    let expires_at_ms = crate::session::unix_epoch_ms().saturating_add(60_000);
    let (session_id, publication_id, source_digest) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut workflow = crate::session::WorkflowDefinition::new(
            "workflow-public-api",
            Some("public-api".to_string()),
        );
        workflow.add_node(crate::session::WorkflowNodeDefinition::new(
            "node-public-api",
            agent.id(),
        ));
        let endpoint = workflow.add_endpoint(crate::session::WorkflowEndpointDefinition::new(
            "endpoint-public-api",
            Some("public-api".to_string()),
            "node-public-api",
        ));
        let source_snapshot = crate::session::WorkflowPublicationSnapshot {
            schema_version: 1,
            captured_at_ms: Some(crate::session::unix_epoch_ms()),
            source_session: Some(crate::session::WorkflowPublicationSourceSessionSnapshot {
                id: Some(session.id().to_string()),
                alias: session.alias().map(str::to_string),
                workspace_id: crate::session::WORKFLOW_PUBLICATION_WORKSPACE_ROOT.to_string(),
                worktree_id: crate::session::WORKFLOW_PUBLICATION_WORKSPACE_ROOT.to_string(),
            }),
            workflow: workflow.clone(),
            endpoint: Some(endpoint),
            queues: Vec::new(),
            schedules: Vec::new(),
            agents: vec![agent.canonicalized_for_publication_package(
                crate::session::WORKFLOW_PUBLICATION_WORKSPACE_ROOT,
            )],
        };
        let source_digest = source_snapshot
            .digest()
            .expect("publication source should hash");
        let mut publication = crate::session::WorkflowPublicationDefinition::new_immutable(
            "publication-public-api",
            session.id(),
            workflow.id(),
            "endpoint-public-api",
            None,
            Some("public-api".to_string()),
            "ingress",
            Some("/public-api".to_string()),
            vec!["POST".to_string()],
            None,
            None,
            None,
            None,
            Some("async".to_string()),
            None,
            None,
            workflow.revision(),
            source_digest.clone(),
            Some("bootstrap-publication".to_string()),
            Some("sha256:bootstrap-request".to_string()),
            "owner-user",
        );
        let open_url = format!(
            "https://relay.example.test/display/{tunnel_id}/publications/runtime/public-api"
        );
        publication.mark_served(
            "running",
            &open_url,
            serde_json::json!({
                "kind": "tunnel",
                "url": open_url,
                "local_url": local_url,
                "runtime_session_id": "publication-runtime-session",
                "expires_at_ms": expires_at_ms,
            }),
        );
        app.sessions
            .write()
            .restore_workflow_publication(session.id(), publication.clone(), Some(source_snapshot))
            .expect("publication should restore into the live session");
        let session = app
            .sessions()
            .get_session(session.id())
            .expect("updated session should load");
        app.durable_state_store()
            .append_event(
                "session.updated",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": &session,
                    "reason": "test_publication_tunnel_restart",
                }),
            )
            .expect("publication session should persist");
        (
            session.id().to_string(),
            publication.id().to_string(),
            source_digest,
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_snapshot = app
        .sessions()
        .resolve_workflow_publication_snapshot(&session_id, &publication_id)
        .expect("restored publication snapshot lookup should succeed")
        .expect("restored publication should retain immutable source");
    assert_eq!(
        restored_snapshot
            .digest()
            .expect("restored publication source should hash"),
        source_digest
    );
    let relay_state = app
        .relay_client_state
        .try_read()
        .expect("bootstrap should release relay state");
    let target = relay_state
        .display_tunnel(tunnel_id, crate::session::unix_epoch_ms())
        .expect("unexpired publication tunnel should restore before relay reconnect");
    assert_eq!(
        target.slice_id,
        format!("publication:{session_id}:{publication_id}")
    );
    assert_eq!(
        target.kind.local_base_url(),
        Some("http://127.0.0.1:43100/")
    );
    assert_eq!(target.expires_at_ms, expires_at_ms);
    assert_eq!(
        target.capabilities,
        vec!["http".to_string(), "publication".to_string()]
    );
}

#[test]
fn bootstrap_restores_queued_prompt_private_state_and_replays_submission_once() {
    let config = DaemonConfig::for_tests();
    let (session_id, agent_id, attachment_id, queued_prompt_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-durable-prompt",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = PromptQueueItem::new(
            "prompt-draft",
            attachment.id(),
            agent.id(),
            "durable queued prompt",
            PromptStatus::Queued,
        )
        .with_hidden_system_context("private durable context")
        .with_durable_operation("command-durable-prompt", "fingerprint-durable-prompt");
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, true)
            .expect("prompt should queue durably");
        let queued_prompt_id = match outcome {
            PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            other => panic!("expected queued prompt, got {other:?}"),
        };
        let later_session = app
            .sessions()
            .get_session(session.id())
            .expect("later session state should load");
        app.durable_state_store()
            .append_event(
                "session.updated",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": &later_session,
                    "reason": "test_later_generic_update",
                }),
            )
            .expect("later generic session event should persist");
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
            queued_prompt_id,
        )
    };

    let mut app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    let queued = restored
        .queued_prompts_for_agent(&agent_id)
        .expect("agent prompt queue should restore");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id(), queued_prompt_id);
    assert_eq!(queued[0].hidden_system_context(), "private durable context");
    assert_eq!(
        queued[0].durable_operation_id(),
        Some("command-durable-prompt")
    );

    let replay = PromptQueueItem::new(
        "prompt-retry-draft",
        attachment_id,
        &agent_id,
        "durable queued prompt",
        PromptStatus::Queued,
    )
    .with_hidden_system_context("private durable context")
    .with_durable_operation("command-durable-prompt", "fingerprint-durable-prompt");
    let replayed = app
        .prompt_owner_submit_prepared_prompt(&session_id, replay, true)
        .expect("matching command should replay despite stale attachment identity");
    match replayed {
        PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), queued_prompt_id),
        other => panic!("expected replayed queued prompt, got {other:?}"),
    }
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(&session_id, &agent_id)
            .expect("queue count should load"),
        1
    );
}

#[test]
fn bootstrap_restores_ended_session_without_live_agents() {
    let config = DaemonConfig::for_tests();
    let session_id = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("agent should spawn");
        crate::app::KernelSessionService::new(&mut app)
            .end_session(session.id())
            .expect("session should end");
        session.id().to_string()
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_session = app
        .sessions()
        .get_session(&session_id)
        .expect("ended session should restore");
    assert_eq!(restored_session.status(), SessionStatus::Ended);
    assert!(
        app.agents.get_session_agents(&session_id).is_empty(),
        "ended sessions should not restore live agents"
    );
}

#[test]
fn bootstrap_restores_snapshot_then_replays_later_events() {
    let config = DaemonConfig::for_tests();
    let (session_id, reviewer_agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        app.save_durable_state_snapshot()
            .expect("snapshot should save");
        let reviewer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("post-snapshot agent should spawn");
        (session.id().to_string(), reviewer.id().to_string())
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    app.sessions()
        .get_session(&session_id)
        .expect("snapshot session should restore");
    assert_eq!(
        app.agents
            .get_agent(&reviewer_agent_id)
            .expect("post-snapshot event should replay")
            .session_id(),
        session_id
    );
}

#[test]
fn bootstrap_restores_metaagent_events_from_snapshot_then_replays_state() {
    let config = DaemonConfig::for_tests();
    let (metaagent_id, event_id, subscription_id, deleted_subscription_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let metaagent = app
            .agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("agent should enter meta mode");
        let metaagent_id = metaagent.id().to_string();
        let event = app.metaagent_event_store().record(
            crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session.id().to_string(),
                metaagent_id: metaagent_id.clone(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: None,
                title: "Worker completed".to_string(),
                summary: "Worker completed a turn".to_string(),
                detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                injected_prompt_id: Some("prompt-meta-1".to_string()),
            },
        );
        app.durable_state_store()
            .append_event(
                "metaagent.event.recorded",
                Some(event.event_id.clone()),
                serde_json::json!({ "record": &event }),
            )
            .expect("event record should persist");
        app.save_durable_state_snapshot()
            .expect("snapshot should save recorded event");

        let delivered = app
            .metaagent_event_store()
            .update_prompt_delivery_status(
                &event.event_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued,
                None,
            )
            .expect("event delivery status should update");
        app.durable_state_store()
            .append_event(
                "metaagent.event.delivery_updated",
                Some(delivered.event_id.clone()),
                serde_json::json!({ "record": &delivered }),
            )
            .expect("event delivery update should persist");

        let read = app
            .metaagent_event_store()
            .read(&metaagent_id, &event.event_id)
            .expect("event should read");
        app.durable_state_store()
            .append_event(
                "metaagent.event.read",
                Some(read.event_id.clone()),
                serde_json::json!({ "record": &read }),
            )
            .expect("event read should persist");

        let acked = app
            .metaagent_event_store()
            .ack(&metaagent_id, &[event.event_id.clone()], None);
        let acked_event = acked.first().expect("event should ack");
        app.durable_state_store()
            .append_event(
                "metaagent.event.acked",
                Some(acked_event.event_id.clone()),
                serde_json::json!({ "record": acked_event }),
            )
            .expect("event ack should persist");

        let subscription = app.metaagent_event_store().subscribe(
            &metaagent_id,
            "workflow.run.completed".to_string(),
            None,
        );
        app.durable_state_store()
            .append_event(
                "metaagent.subscription.created",
                Some(subscription.subscription_id.clone()),
                serde_json::json!({ "subscription": &subscription }),
            )
            .expect("subscription should persist");

        let deleted_subscription = app.metaagent_event_store().subscribe(
            &metaagent_id,
            "workflow.run.failed".to_string(),
            None,
        );
        app.durable_state_store()
            .append_event(
                "metaagent.subscription.created",
                Some(deleted_subscription.subscription_id.clone()),
                serde_json::json!({ "subscription": &deleted_subscription }),
            )
            .expect("deleted subscription create should persist");
        let deleted_subscription = app
            .metaagent_event_store()
            .unsubscribe(&metaagent_id, &deleted_subscription.subscription_id)
            .expect("subscription should remove");
        app.durable_state_store()
            .append_event(
                "metaagent.subscription.deleted",
                Some(deleted_subscription.subscription_id.clone()),
                serde_json::json!({ "subscription": &deleted_subscription }),
            )
            .expect("subscription deletion should persist");

        (
            metaagent_id,
            event.event_id,
            subscription.subscription_id,
            deleted_subscription.subscription_id,
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_events = app.metaagent_event_store().list(
        &metaagent_id,
        Some("agent.turn.completed"),
        Some("acked"),
        10,
    );
    assert_eq!(restored_events.len(), 1);
    assert_eq!(restored_events[0].event_id, event_id);
    assert!(restored_events[0].read_at_ms.is_some());
    assert!(restored_events[0].ack_at_ms.is_some());
    assert_eq!(
        restored_events[0].injected_prompt_id.as_deref(),
        Some("prompt-meta-1")
    );
    assert_eq!(
        restored_events[0].prompt_delivery_status,
        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
    );
    assert!(restored_events[0].prompt_delivery_updated_at_ms.is_some());

    let restored_subscriptions = app
        .metaagent_event_store()
        .list_subscriptions(&metaagent_id);
    assert_eq!(restored_subscriptions.len(), 1);
    assert_eq!(restored_subscriptions[0].subscription_id, subscription_id);
    assert_ne!(
        restored_subscriptions[0].subscription_id,
        deleted_subscription_id
    );
}

#[test]
fn bootstrap_preserves_durable_runtime_work_for_restart_recovery() {
    let config = DaemonConfig::for_tests();
    let (session_id, workflow_run_id, workflow_node_run_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let mut session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let session_id = session.id().to_string();
        let workflow_run_id = "workflow-run-stale".to_string();
        let workflow_node_run_id = "node-run-stale".to_string();
        session.set_active_provider_run(Some("provider-run-stale".to_string()));
        let node_run = WorkflowNodeRun::new(
            &workflow_node_run_id,
            "node-1",
            agent.id(),
            1,
            WorkflowNodeRunStatus::Running,
        );
        let mut workflow_run = WorkflowRun::new(
            &workflow_run_id,
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("invoke".to_string()),
            None,
            vec![node_run],
            Vec::new(),
        );
        workflow_run.set_active_node_run(&workflow_node_run_id);
        workflow_run.set_status(WorkflowRunStatus::Running);
        session.create_workflow_run(workflow_run);
        app.sessions.restore_session(session);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(
                &session_id,
                PromptQueueItem::new(
                    "prompt-workflow-stale",
                    format!("workflow-run:{workflow_run_id}"),
                    agent.id(),
                    "still running when the kernel stops",
                    PromptStatus::Queued,
                )
                .with_workflow_context(&workflow_run_id, &workflow_node_run_id),
                false,
            )
            .expect("workflow prompt should start");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        app.save_durable_state_snapshot()
            .expect("snapshot should save stale runtime state");
        (session_id, workflow_run_id, workflow_node_run_id)
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert_eq!(restored.active_provider_run_id(), None);
    assert_eq!(
        restored.active_prompt().map(|prompt| prompt.prompt()),
        Some("still running when the kernel stops")
    );
    assert_eq!(restored.scheduler_state(), SchedulerState::Running);
    let workflow_run = restored
        .workflow_run(&workflow_run_id)
        .expect("workflow run should restore");
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Running);
    assert_eq!(workflow_run.active_node_run_id(), Some("node-run-stale"));
    assert_eq!(
        workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Running
    );
    assert_eq!(workflow_run.node_runs()[0].id(), workflow_node_run_id);
    assert!(!workflow_run
        .failure_events()
        .iter()
        .any(|event| { event.message().contains("interrupted by kernel restart") }));
}

#[test]
fn bootstrap_stops_orphaned_prepared_workflow_run_after_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, workflow_run_id, workflow_node_run_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let mut session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let session_id = session.id().to_string();
        let mut node_run = WorkflowNodeRun::new(
            "node-run-prepared",
            "node-1",
            agent.id(),
            1,
            WorkflowNodeRunStatus::Ready,
        );
        node_run.set_turn_envelope(Some(WorkflowTurnEnvelope::new(
            "workflow-ack:node-run-prepared",
            "assembled prompt".to_string(),
            None,
            None,
        )));
        let mut workflow_run = WorkflowRun::new(
            "workflow-run-prepared",
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("invoke".to_string()),
            None,
            vec![node_run],
            Vec::new(),
        );
        workflow_run.set_active_node_run("node-run-prepared");
        session.create_workflow_run(workflow_run);
        app.sessions.restore_session(session);
        app.save_durable_state_snapshot()
            .expect("snapshot should save prepared runtime state");
        (
            session_id,
            "workflow-run-prepared".to_string(),
            "node-run-prepared".to_string(),
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert!(!restored.has_active_workflow_run());
    let workflow_run = restored
        .workflow_run(&workflow_run_id)
        .expect("workflow run should restore");
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Stopped);
    assert_eq!(workflow_run.active_node_run_id(), None);
    let node_run = &workflow_run.node_runs()[0];
    assert_eq!(node_run.id(), workflow_node_run_id);
    assert_eq!(node_run.status(), WorkflowNodeRunStatus::Stopped);
    assert_eq!(
        node_run
            .turn_envelope()
            .expect("turn envelope should remain visible")
            .state(),
        crate::session::WorkflowTurnRuntimeState::Cancelled
    );
    assert!(workflow_run.failure_events().iter().any(|event| {
        event
            .message()
            .contains("no durable active or queued prompt")
    }));
}

#[test]
fn bootstrap_preserves_prepared_workflow_run_with_durable_prompt_after_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, workflow_run_id, workflow_node_run_id, prompt_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let mut session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let session_id = session.id().to_string();
        let workflow_run_id = "workflow-run-prepared".to_string();
        let workflow_node_run_id = "node-run-prepared".to_string();
        let prompt_draft_id = "prompt-workflow-prepared".to_string();
        let mut node_run = WorkflowNodeRun::new(
            &workflow_node_run_id,
            "node-1",
            agent.id(),
            1,
            WorkflowNodeRunStatus::Ready,
        );
        node_run.set_turn_envelope(Some(WorkflowTurnEnvelope::new(
            "workflow-ack:node-run-prepared",
            "assembled prompt".to_string(),
            None,
            None,
        )));
        let mut workflow_run = WorkflowRun::new(
            &workflow_run_id,
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("invoke".to_string()),
            None,
            vec![node_run],
            Vec::new(),
        );
        workflow_run.set_active_node_run(&workflow_node_run_id);
        session.create_workflow_run(workflow_run);
        app.sessions.restore_session(session);
        let outcome = app
            .prompt_owner_submit_prepared_prompt(
                &session_id,
                PromptQueueItem::new(
                    &prompt_draft_id,
                    "workflow-run:workflow-run-prepared",
                    agent.id(),
                    "assembled prompt",
                    PromptStatus::Queued,
                )
                .with_workflow_context(&workflow_run_id, &workflow_node_run_id),
                false,
            )
            .expect("workflow prompt should persist");
        let prompt_id = match outcome {
            PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            other => panic!("expected started workflow prompt, got {other:?}"),
        };
        app.save_durable_state_snapshot()
            .expect("snapshot should save prepared runtime state");
        (session_id, workflow_run_id, workflow_node_run_id, prompt_id)
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert_eq!(
        restored.active_prompt().map(|prompt| prompt.id()),
        Some(prompt_id.as_str())
    );
    assert!(restored.has_active_workflow_run());
    let workflow_run = restored
        .workflow_run(&workflow_run_id)
        .expect("workflow run should restore");
    // Bootstrap recovery promotes a durable prepared prompt into the active
    // workflow turn before the listener is ready. The persisted run is
    // therefore intentionally Running/Dispatched after restart, not left in
    // its pre-admission Created/Prepared projection.
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Running);
    assert_eq!(
        workflow_run.active_node_run_id(),
        Some(workflow_node_run_id.as_str())
    );
    let node_run = &workflow_run.node_runs()[0];
    assert_eq!(node_run.status(), WorkflowNodeRunStatus::Running);
    assert_eq!(
        node_run
            .turn_envelope()
            .expect("turn envelope should remain visible")
            .state(),
        crate::session::WorkflowTurnRuntimeState::Dispatched
    );
    assert!(workflow_run.failure_events().is_empty());
}
