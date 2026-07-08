use std::time::Instant;

use super::{
    AgentPromptRuntimeStatus, AgentRuntimeStatus, AgentTurnRuntimePhase, SessionSnapshotProjection,
};
use crate::agent::CreateAgentRequest;
use crate::runtime::projection::{
    test_support::{attach_cli, launch_dev_stub_provider, submit_prompt},
    QUEUED_PROMPT_STEER_EXTERNAL_REASON,
};
use crate::session::CreateSessionRequest;
use crate::{DaemonApp, DaemonConfig};

#[test]
fn session_snapshot_projection_includes_metadata_agents_and_idle_activity() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let reviewer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
        .expect("reviewer should be created");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");

    assert_eq!(
        projection.metadata.projection_version,
        super::SESSION_SNAPSHOT_PROJECTION_VERSION
    );
    assert_eq!(projection.metadata.last_event_id, 42);
    assert_eq!(projection.session.id(), session.id());
    assert_eq!(projection.session.agents().len(), 2);
    assert_eq!(projection.agent_activity.len(), 2);
    for agent_id in [agent.id(), reviewer.id()] {
        let activity = projection
            .agent_activity
            .get(agent_id)
            .expect("every visible agent should have projected runtime activity");
        assert_eq!(activity.status, AgentRuntimeStatus::Idle);
        assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
        assert!(!activity.busy);
        assert_eq!(activity.active_prompt_count, 0);
        assert_eq!(activity.queued_prompt_count, 0);
        assert!(activity.active_turn.is_none());
    }
}

#[test]
fn session_snapshot_projection_uses_projected_provider_run_fallback() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let mut provider_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "projected-run",
        session.id().to_string(),
        Some(agent.id().to_string()),
        "codex".to_string(),
    );
    provider_run.mark_running();
    provider_run.set_usage(crate::provider::ProviderRunTokenUsage {
        total_tokens: Some(42),
        last_tokens: Some(42),
        context_tokens: Some(42),
        context_window: Some(128_000),
    });
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(provider_run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(provider_run.clone());
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-projected".to_string(),
            provider_run.id().to_string(),
        ));

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let projected_run = projection
        .provider_run
        .as_ref()
        .expect("provider run should be projected from fallback");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(
        projection.session.active_provider_run_id(),
        Some(provider_run.id())
    );
    assert_eq!(projected_run.id(), provider_run.id());
    assert_eq!(projected_run.usage(), provider_run.usage());
    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert_eq!(
        activity
            .active_turn
            .as_ref()
            .and_then(|turn| turn.provider_run_id.as_deref()),
        Some(provider_run.id())
    );
}

#[test]
fn session_snapshot_projection_preserves_projected_active_run_with_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let focused_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("focused"))
        .expect("focused agent should be created");

    app.sessions_mut()
        .set_focused_agent(session.id(), Some(focused_agent.id().to_string()))
        .expect("focused agent should be set");
    for agent in [&first_agent, &focused_agent] {
        let external_prompt = crate::session::PromptQueueItem::external_observed_running(
            "codex",
            "session-1",
            format!("{}-turn", agent.id()),
            agent.id(),
            format!("external prompt for {}", agent.id()),
        );
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
        .expect("external active prompt should sync");
    }

    let mut first_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "projected-run-first",
        session.id().to_string(),
        Some(first_agent.id().to_string()),
        "codex".to_string(),
    );
    first_run.mark_running();
    let mut focused_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "projected-run-focused",
        session.id().to_string(),
        Some(focused_agent.id().to_string()),
        "codex".to_string(),
    );
    focused_run.mark_running();
    app.update_provider_run_projection(first_run.clone());
    app.update_provider_run_projection(focused_run);
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(first_run.id().to_string()))
        .expect("active provider run should be set");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let projected_run = projection
        .provider_run
        .as_ref()
        .expect("provider run should be projected from fallback");

    assert_eq!(
        projection.session.active_provider_run_id(),
        Some(first_run.id())
    );
    assert_eq!(projected_run.id(), first_run.id());
    assert_eq!(
        projection
            .agent_activity
            .get(first_agent.id())
            .expect("first agent activity should project")
            .status,
        AgentRuntimeStatus::Working
    );
    assert_eq!(
        projection
            .agent_activity
            .get(focused_agent.id())
            .expect("focused agent activity should project")
            .status,
        AgentRuntimeStatus::Working
    );
}

#[test]
fn session_snapshot_projection_marks_settling_prompt_as_working() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-settling");
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "status check",
    );
    app.prompt_activity_store().write().insert(
        provider_run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
        },
    );
    let active_turns = app.active_turn_store();
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().to_string(),
        agent.id().to_string(),
        "prompt-settling".to_string(),
        provider_run.id().to_string(),
    ));
    active_turns.mark_settling(provider_run.id());

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Settling);
    assert!(activity.busy);
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("settling prompt should project active turn");
    assert_eq!(active_turn.phase, AgentTurnRuntimePhase::Settling);
    assert!(active_turn.started_at_ms.is_some());
}

#[test]
fn session_snapshot_projection_keeps_active_turn_working_without_active_prompt_activity() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-restored".to_string(),
            provider_run.id().to_string(),
        )
        .with_phase(crate::app::ActiveTurnPhase::AwaitingFirstOutput),
    );

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert!(activity.busy);
    assert_eq!(
        activity
            .active_turn
            .as_ref()
            .map(|turn| turn.prompt_id.as_str()),
        Some("prompt-restored")
    );
    assert_eq!(
        activity.active_turn.as_ref().map(|turn| &turn.phase),
        Some(&AgentTurnRuntimePhase::AwaitingFirstOutput)
    );
}

#[test]
fn session_snapshot_projection_does_not_infer_external_origin_from_active_turn_prompt_id() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "external:codex:session-1:user-1".to_string(),
            provider_run.id().to_string(),
        )
        .with_phase(crate::app::ActiveTurnPhase::Streaming),
    );

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let active_turn = projection
        .agent_activity
        .get(agent.id())
        .and_then(|activity| activity.active_turn.as_ref())
        .expect("active turn should still project as runtime activity");

    assert_eq!(
        active_turn.prompt_id,
        "external:codex:session-1:user-1".to_string()
    );
    assert_eq!(active_turn.prompt_origin, None);
    assert_eq!(active_turn.external_provider, None);
    assert_eq!(active_turn.external_provider_session_id, None);
    assert_eq!(active_turn.external_provider_turn_id, None);
}

#[test]
fn session_snapshot_active_turn_phase_drill_projects_awaiting_first_output() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-awaiting-output");
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "status check",
    );
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("active turn should be projected before first output");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert_eq!(
        active_turn.phase,
        AgentTurnRuntimePhase::AwaitingFirstOutput
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::Arroba)
    );
    assert!(active_turn.started_at_ms.is_some());
}

#[test]
fn session_snapshot_projection_projects_external_active_turn_origin() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "session-1",
        "user-1",
        agent.id(),
        "external prompt",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "external:codex:session-1:user-1".to_string(),
            provider_run.id().to_string(),
        ));

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("external active turn should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(active_turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        active_turn.external_provider_session_id.as_deref(),
        Some("session-1")
    );
    assert_eq!(
        active_turn.external_provider_turn_id.as_deref(),
        Some("user-1")
    );
}

#[test]
fn session_snapshot_projection_matches_active_turn_by_pending_prompt_id() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let active_prompt: crate::session::PromptQueueItem =
        serde_json::from_value(serde_json::json!({
            "id": "prompt-real-1",
            "pending_prompt_id": "pending-prompt-1",
            "source_attachment_id": "external:codex",
            "target_agent_id": agent.id(),
            "prompt": "external prompt",
            "attachments": [],
            "status": "Running",
            "prompt_origin": "external",
            "external_provider": "codex",
            "external_provider_session_id": "thread-1",
            "external_provider_turn_id": "user-1"
        }))
        .expect("active prompt should deserialize");
    app.prompt_owner_activate_prompt(session.id(), active_prompt)
        .expect("active prompt should activate");
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "pending-prompt-1".to_string(),
            provider_run.id().to_string(),
        ));

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let active_turn = projection
        .agent_activity
        .get(agent.id())
        .and_then(|activity| activity.active_turn.as_ref())
        .expect("active turn should be projected");

    assert_eq!(active_turn.prompt_id, "pending-prompt-1");
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(active_turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        active_turn.external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        active_turn.external_provider_turn_id.as_deref(),
        Some("user-1")
    );
}

#[test]
fn session_snapshot_projection_keeps_external_active_turn_origin_without_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "session-1",
        "user-1",
        agent.id(),
        "external prompt",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    crate::transport::flow_control::note_prompt_started(&mut app, provider_run.id());
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
        .expect("external active prompt should clear");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("external active turn should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert_eq!(
        active_turn.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(active_turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        active_turn.external_provider_session_id.as_deref(),
        Some("session-1")
    );
    assert_eq!(
        active_turn.external_provider_turn_id.as_deref(),
        Some("user-1")
    );
}

#[test]
fn session_snapshot_projection_projects_queued_prompt_controls() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-queued-controls");
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "active prompt",
    );
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "queued prompt",
    );

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let control = activity
        .queued_prompt_controls
        .values()
        .next()
        .expect("queued prompt control should be projected");

    assert_eq!(activity.active_prompt_count, 1);
    assert_eq!(activity.queued_prompt_count, 1);
    assert_eq!(control.status, "queued");
    assert!(control.can_steer);
    assert!(control.can_cancel);
    assert!(control.steer_disabled_reason.is_none());
    assert!(control.cancel_disabled_reason.is_none());
}

#[test]
fn session_snapshot_projection_blocks_steering_behind_explicit_external_active_turn() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "session-1",
        "user-1",
        agent.id(),
        "external prompt",
    );
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            external_prompt.id().to_string(),
            provider_run.id().to_string(),
        )
        .with_prompt_metadata(&external_prompt)
        .with_phase(crate::app::ActiveTurnPhase::Streaming),
    );
    let attachment_id = attach_cli(&mut app, session.id(), "cli-sparse-external-queue");
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "queued-behind-external",
            &attachment_id,
            agent.id(),
            "queued prompt",
            crate::session::PromptStatus::Queued,
        ),
        true,
    )
    .expect("prompt should queue behind sparse turn");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("active turn should be projected");
    let control = activity
        .queued_prompt_controls
        .values()
        .next()
        .expect("queued prompt control should be projected");

    assert_eq!(activity.active_prompt_count, 1);
    assert_eq!(activity.queued_prompt_count, 1);
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(control.status, "queued");
    assert!(!control.can_steer);
    assert!(control.can_cancel);
    assert_eq!(
        control.steer_disabled_reason.as_deref(),
        Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON)
    );
    assert!(control.cancel_disabled_reason.is_none());
}

#[test]
fn session_snapshot_projection_projects_active_turn_when_provider_run_lookup_is_cold() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "session-1",
        "user-1",
        agent.id(),
        "external prompt",
    );
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            external_prompt.id().to_string(),
            "cold-provider-run".to_string(),
        )
        .with_prompt_metadata(&external_prompt)
        .with_phase(crate::app::ActiveTurnPhase::Streaming),
    );
    let attachment_id = attach_cli(&mut app, session.id(), "cli-cold-active-turn");
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "queued-behind-cold-external",
            &attachment_id,
            agent.id(),
            "queued prompt",
            crate::session::PromptStatus::Queued,
        ),
        true,
    )
    .expect("prompt should queue behind cold external turn");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let active_turn = activity
        .active_turn
        .as_ref()
        .expect("cold active turn should be projected");
    let control = activity
        .queued_prompt_controls
        .values()
        .next()
        .expect("queued prompt control should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Running);
    assert_eq!(activity.active_prompt_count, 1);
    assert_eq!(
        active_turn.provider_run_id.as_deref(),
        Some("cold-provider-run")
    );
    assert_eq!(
        active_turn.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(active_turn.external_provider.as_deref(), Some("codex"));
    assert!(!control.can_steer);
    assert!(control.can_cancel);
    assert_eq!(
        control.steer_disabled_reason.as_deref(),
        Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON)
    );
}

#[test]
fn session_snapshot_projection_keeps_queued_only_prompts_idle() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment_id = attach_cli(&mut app, session.id(), "cli-queued-only");
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        crate::session::PromptQueueItem::new(
            "queued-only",
            &attachment_id,
            agent.id(),
            "queued prompt",
            crate::session::PromptStatus::Queued,
        ),
        true,
    )
    .expect("prompt should queue");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Idle);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::Queued);
    assert!(!activity.busy);
    assert_eq!(activity.active_prompt_count, 0);
    assert_eq!(activity.queued_prompt_count, 1);
    assert_eq!(activity.queued_prompt_controls.len(), 1);
}

#[test]
fn session_snapshot_projection_marks_dispatching_prompt_as_active_work() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-dispatching");
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "active prompt",
    );
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "queued prompt",
    );
    let pending = app
        .prompt_owner_peek_next_queued_prompt(session.id(), agent.id())
        .expect("queue peek should succeed")
        .expect("queued prompt should exist");
    app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
        .expect("active prompt should complete");
    app.prompt_owner_activate_next_queued_prompt_with_prompt_id(
        session.id(),
        agent.id(),
        Some(pending.id()),
        "prompt-dispatching".to_string(),
    )
    .expect("queued prompt should activate");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Working);
    assert_eq!(
        activity.prompt_status,
        AgentPromptRuntimeStatus::Dispatching
    );
    assert!(activity.busy);
    assert_eq!(activity.active_prompt_count, 1);
    assert_eq!(
        activity.active_turn.as_ref().map(|turn| &turn.status),
        Some(&AgentPromptRuntimeStatus::Dispatching)
    );
}

#[test]
fn session_snapshot_projection_disables_queued_prompt_steering_for_external_active_turns() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-external-queued-controls");
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "session-1",
        "user-1",
        agent.id(),
        "external prompt",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "external:codex:session-1:user-1".to_string(),
            provider_run.id().to_string(),
        ));
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "queued behind external prompt",
    );

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");
    let control = activity
        .queued_prompt_controls
        .values()
        .next()
        .expect("queued prompt control should be projected");

    assert_eq!(activity.active_prompt_count, 1);
    assert_eq!(activity.queued_prompt_count, 1);
    assert_eq!(control.status, "queued");
    assert!(!control.can_steer);
    assert!(control.can_cancel);
    assert_eq!(
        control.steer_disabled_reason.as_deref(),
        Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON)
    );
    assert!(control.cancel_disabled_reason.is_none());
}

#[test]
fn session_snapshot_projection_ignores_prompt_activity_without_active_turn() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    app.prompt_activity_store().write().insert(
        provider_run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: false,
            settlement_requested: true,
        },
    );

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Idle);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
    assert!(!activity.busy);
    assert!(activity.active_turn.is_none());
}

#[test]
fn session_snapshot_projection_marks_completed_prompt_as_idle() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
    let attachment_id = attach_cli(&mut app, session.id(), "cli-idle");
    submit_prompt(
        &mut app,
        session.id(),
        &attachment_id,
        agent.id(),
        "status check",
    );
    app.complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
        .expect("prompt should complete");

    let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
        .expect("projection should build");
    let activity = projection
        .agent_activity
        .get(agent.id())
        .expect("agent activity should be projected");

    assert_eq!(activity.status, AgentRuntimeStatus::Idle);
    assert_eq!(activity.prompt_status, AgentPromptRuntimeStatus::None);
    assert!(!activity.busy);
}
