use std::collections::{BTreeMap, VecDeque};

use super::prompt_matches_active_turn;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::runtime::projection::test_support::{launch_dev_stub_provider, submit_prompt};
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
use crate::session::{CreateSessionRequest, PromptOrigin, PromptQueueItem};
use crate::{DaemonApp, DaemonConfig};

#[test]
fn projection_invariant_health_reports_agent_runtime_drift() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            &session_id,
            "cli-projection-invariant",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, &session_id, &agent_id);
    submit_prompt(
        &mut app,
        &session_id,
        attachment.id(),
        &agent_id,
        "active prompt",
    );
    submit_prompt(
        &mut app,
        &session_id,
        attachment.id(),
        &agent_id,
        "queued prompt",
    );

    let session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should load");
    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);
    let active_turns = BTreeMap::new();
    let provider_runs = Vec::new();
    let clean_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert_eq!(clean_snapshot.checked_sessions, 1);
    assert_eq!(clean_snapshot.checked_agents, 1);
    assert!(clean_snapshot.mismatches.is_empty());

    let projection = agent_store
        .get(&agent_id)
        .expect("agent projection should exist before drift injection");
    agent_store.update_agent_prompt_state(
        &session_id,
        &agent_id,
        projection.active_prompt,
        None,
        0,
    );

    let drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(drift_snapshot
        .mismatches
        .iter()
        .any(|mismatch| mismatch.kind == "queue_front_mismatch"));
    assert!(drift_snapshot
        .mismatches
        .iter()
        .any(|mismatch| mismatch.kind == "queued_prompt_count_mismatch"));
}

#[test]
fn projection_invariant_health_reports_stale_focused_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let mut session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should load");
    session.set_focused_agent(Some("missing-agent".to_string()));

    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);

    let active_turns = BTreeMap::new();
    let provider_runs = Vec::new();
    let snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

    assert_eq!(snapshot.checked_sessions, 1);
    assert_eq!(snapshot.checked_agents, 1);
    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "stale_focused_agent"
            && mismatch.session_id == session.id()
            && mismatch.agent_id.as_deref() == Some("missing-agent")
            && mismatch.details == "focused agent is not present in the session agent list"
    }));
    assert!(!snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "missing_agent_runtime_projection"
            && mismatch.agent_id.as_deref() == Some(agent.id())
    }));
}

#[test]
fn projection_invariant_health_reports_prompt_state_without_session_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            &session_id,
            "cli-projection-invariant-prompt-state-only",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, &session_id, &agent_id);
    submit_prompt(
        &mut app,
        &session_id,
        attachment.id(),
        &agent_id,
        "active prompt",
    );

    let mut session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should load");
    assert!(
        session.prompt_states().contains_key(&agent_id),
        "fixture should retain prompt state before removing projected agents"
    );
    session.set_agents(Vec::new());

    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);

    let active_turns = BTreeMap::new();
    let provider_runs = Vec::new();
    let snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

    assert_eq!(snapshot.checked_sessions, 1);
    assert_eq!(snapshot.checked_agents, 0);
    assert!(agent_store.get(&agent_id).is_none());
    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "prompt_state_without_session_agent"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details == "session prompt state has no matching session agent"
    }));
    assert!(!snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "orphaned_agent_runtime_projection"
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
    }));
}

#[test]
fn projection_invariant_health_reports_prompt_target_drift_inside_prompt_state() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let wrong_agent_id = "agent-other";
    let active_prompt = PromptQueueItem::new(
        "prompt-active-wrong-target",
        "attachment-1",
        wrong_agent_id,
        "active prompt",
        crate::session::PromptStatus::Running,
    );
    let queued_prompt = PromptQueueItem::new(
        "prompt-queued-wrong-target",
        "attachment-1",
        wrong_agent_id,
        "queued prompt",
        crate::session::PromptStatus::Queued,
    );
    let session = app
        .sessions
        .mirror_agent_prompt_state(
            &session_id,
            &agent_id,
            Some(active_prompt),
            VecDeque::from([queued_prompt]),
        )
        .expect("prompt state should mirror");

    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);

    let active_turns = BTreeMap::new();
    let provider_runs = Vec::new();
    let snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "prompt_state_prompt_target_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("active")
            && mismatch.details.contains("prompt-active-wrong-target")
            && mismatch.details.contains(wrong_agent_id)
    }));
    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "prompt_state_prompt_target_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("queued")
            && mismatch.details.contains("prompt-queued-wrong-target")
            && mismatch.details.contains(wrong_agent_id)
    }));
}

#[test]
fn projection_invariant_health_reports_active_turn_membership_drift() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let valid_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let wrong_agent_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let mut ended_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    ended_run.mark_ended();
    let session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should load");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_runs = vec![
        valid_run.clone(),
        wrong_agent_run.clone(),
        ended_run.clone(),
    ];

    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);
    let mut active_turns = BTreeMap::new();
    active_turns.insert(
        valid_run.id().to_string(),
        crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            "prompt-valid".to_string(),
            valid_run.id().to_string(),
        ),
    );

    let clean_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(
        clean_snapshot.mismatches.is_empty(),
        "{:?}",
        clean_snapshot.mismatches
    );

    active_turns.insert(
        "run-missing-session".to_string(),
        crate::app::ActiveTurnState::new(
            "missing-session".to_string(),
            agent_id.clone(),
            "prompt-missing-session".to_string(),
            "run-missing-session".to_string(),
        ),
    );
    active_turns.insert(
        wrong_agent_run.id().to_string(),
        crate::app::ActiveTurnState::new(
            session_id.clone(),
            "missing-agent".to_string(),
            "prompt-missing-agent".to_string(),
            wrong_agent_run.id().to_string(),
        ),
    );
    active_turns.insert(
        ended_run.id().to_string(),
        crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            "prompt-ended-run".to_string(),
            ended_run.id().to_string(),
        ),
    );

    let drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_missing_provider_run"
            && mismatch.session_id == "missing-session"
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("run-missing-session")
    }));
    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_missing_session"
            && mismatch.session_id == "missing-session"
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("run-missing-session")
    }));
    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_missing_session_agent"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some("missing-agent")
            && mismatch.details.contains(wrong_agent_run.id())
    }));
    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_provider_run_agent_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some("missing-agent")
            && mismatch.details.contains(wrong_agent_run.id())
    }));
    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_ended_provider_run"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains(ended_run.id())
    }));
    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "duplicate_active_turns_for_agent"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains(valid_run.id())
            && mismatch.details.contains(ended_run.id())
    }));
}

#[test]
fn projection_invariant_health_reports_active_turn_prompt_identity_drift() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_run = launch_dev_stub_provider(&mut app, &session_id, &agent_id);
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            &session_id,
            "cli-projection-invariant-active-turn",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    submit_prompt(
        &mut app,
        &session_id,
        attachment.id(),
        &agent_id,
        "active prompt",
    );
    let session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should load");
    let active_prompt = session
        .active_prompt_for_agent(&agent_id)
        .expect("active prompt should exist");
    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);
    let provider_runs = vec![provider_run.clone()];
    let mut active_turns = BTreeMap::new();
    active_turns.insert(
        provider_run.id().to_string(),
        crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            active_prompt.id().to_string(),
            provider_run.id().to_string(),
        ),
    );

    let clean_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(
        clean_snapshot.mismatches.is_empty(),
        "{:?}",
        clean_snapshot.mismatches
    );

    active_turns.insert(
        provider_run.id().to_string(),
        crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            "different-prompt".to_string(),
            provider_run.id().to_string(),
        ),
    );
    let drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

    assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_active_prompt_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("different-prompt")
            && mismatch.details.contains(active_prompt.id())
    }));
}

#[test]
fn active_turn_prompt_identity_accepts_pending_prompt_id() {
    let prompt: PromptQueueItem = serde_json::from_value(serde_json::json!({
        "id": "prompt-real-1",
        "pending_prompt_id": "pending-prompt-1",
        "source_attachment_id": "attachment-1",
        "target_agent_id": "agent-1",
        "prompt": "prompt",
        "attachments": [],
        "status": "Running"
    }))
    .expect("prompt should deserialize");

    assert!(prompt_matches_active_turn(&prompt, "prompt-real-1"));
    assert!(prompt_matches_active_turn(&prompt, "pending-prompt-1"));
    assert!(!prompt_matches_active_turn(&prompt, "other-prompt"));
}

#[test]
fn projection_invariant_health_reports_active_turn_prompt_metadata_drift() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_run = launch_dev_stub_provider(&mut app, &session_id, &agent_id);
    let external_prompt = PromptQueueItem::external_observed_running(
        "codex",
        "thread-1",
        "user-1",
        &agent_id,
        "external prompt",
    );
    app.prompt_owner_sync_external_active_prompt(
        &session_id,
        &agent_id,
        Some(external_prompt.clone()),
    )
    .expect("external active prompt should sync");
    let session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should load");
    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(session.clone());
    agent_store.update_session(&session);
    let provider_runs = vec![provider_run.clone()];
    let active_turn = crate::app::ActiveTurnState::new(
        session_id.clone(),
        agent_id.clone(),
        external_prompt.id().to_string(),
        provider_run.id().to_string(),
    )
    .with_prompt_metadata(&external_prompt);
    let mut active_turns = BTreeMap::new();
    active_turns.insert(provider_run.id().to_string(), active_turn.clone());

    let clean_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(
        clean_snapshot.mismatches.is_empty(),
        "{:?}",
        clean_snapshot.mismatches
    );

    let mut wrong_origin = active_turn.clone();
    wrong_origin.prompt_origin = Some(PromptOrigin::Arroba);
    active_turns.insert(provider_run.id().to_string(), wrong_origin);
    let origin_drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(origin_drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_prompt_origin_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("arroba")
            && mismatch.details.contains("external")
    }));

    let mut wrong_source_attachment = active_turn.clone();
    wrong_source_attachment.source_attachment_id = Some("attachment-wrong".to_string());
    active_turns.insert(provider_run.id().to_string(), wrong_source_attachment);
    let source_attachment_drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(
        source_attachment_drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| {
                mismatch.kind == "active_turn_source_attachment_mismatch"
                    && mismatch.session_id == session_id
                    && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                    && mismatch.details.contains("attachment-wrong")
                    && mismatch.details.contains("external:codex")
            })
    );

    let mut missing_source_attachment = active_turn.clone();
    missing_source_attachment.source_attachment_id = None;
    active_turns.insert(provider_run.id().to_string(), missing_source_attachment);
    let missing_source_attachment_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(
        missing_source_attachment_snapshot
            .mismatches
            .iter()
            .any(|mismatch| {
                mismatch.kind == "active_turn_source_attachment_mismatch"
                    && mismatch.session_id == session_id
                    && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                    && mismatch.details.contains("none")
                    && mismatch.details.contains("external:codex")
            })
    );

    let mut missing_origin = active_turn.clone();
    missing_origin.prompt_origin = None;
    active_turns.insert(provider_run.id().to_string(), missing_origin);
    let missing_origin_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(missing_origin_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_prompt_origin_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("none")
            && mismatch.details.contains("external")
    }));

    let mut wrong_external_identity = active_turn.clone();
    wrong_external_identity.external_observed_id =
        crate::history::parse_external_provider_observed_id("external:codex:thread-1:user-2");
    active_turns.insert(provider_run.id().to_string(), wrong_external_identity);
    let identity_drift_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(identity_drift_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_external_identity_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("user-1")
            && mismatch.details.contains("user-2")
    }));

    let mut missing_external_identity = active_turn.clone();
    missing_external_identity.external_observed_id = None;
    active_turns.insert(provider_run.id().to_string(), missing_external_identity);
    let missing_identity_snapshot =
        session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
    assert!(missing_identity_snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "active_turn_external_identity_mismatch"
            && mismatch.session_id == session_id
            && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
            && mismatch.details.contains("none")
            && mismatch.details.contains("user-1")
    }));
}

#[test]
fn projection_invariant_health_reports_canonical_agent_membership_drift() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");
    let second_session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(second_session.id())
        .expect("second session snapshot should load");
    let ghost_agent = crate::agent::AgentInstance::new(
        "ghost-agent",
        "ghost-agent",
        second_session.id(),
        None,
        "dev-stub",
        Some("default".to_string()),
        None,
        Some(second_session.worktree_id().to_string()),
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    let session_store = SessionStateProjectionStore::default();
    let agent_store = AgentRuntimeProjectionStore::default();
    session_store.update(second_session.clone());
    agent_store.update_session(&second_session);

    let active_turns = BTreeMap::new();
    let provider_runs = Vec::new();
    let snapshot = session_store.invariant_snapshot(
        &agent_store,
        &[first_agent.clone(), ghost_agent.clone()],
        &active_turns,
        &provider_runs,
    );

    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "agent_record_missing_projected_session"
            && mismatch.session_id == first_session.id()
            && mismatch.agent_id.as_deref() == Some(first_agent.id())
    }));
    assert!(snapshot.mismatches.iter().any(|mismatch| {
        mismatch.kind == "agent_record_not_in_session_projection"
            && mismatch.session_id == second_session.id()
            && mismatch.agent_id.as_deref() == Some(ghost_agent.id())
    }));
}
