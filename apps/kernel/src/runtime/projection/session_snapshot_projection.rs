use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    queued_prompt_controls_projection, AgentQueuedPromptControlProjection, ProjectionMetadata,
};
use crate::agent::AgentState;
use crate::app::{ActivePromptState, ActiveTurnPhase, ActiveTurnState, DaemonApp};
use crate::error::DaemonError;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::{PromptOrigin, PromptQueueItem, PromptStatus, RuntimeSession};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshotProjection {
    pub metadata: ProjectionMetadata,
    pub session: RuntimeSession,
    pub provider_run: Option<RuntimeProviderRun>,
    pub agent_activity: BTreeMap<String, AgentRuntimeActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStatus {
    Idle,
    Working,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptRuntimeStatus {
    None,
    Queued,
    Dispatching,
    Running,
    Cancelling,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnRuntimePhase {
    Accepted,
    AwaitingFirstOutput,
    Streaming,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeActivity {
    pub status: AgentRuntimeStatus,
    pub prompt_status: AgentPromptRuntimeStatus,
    pub busy: bool,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    #[serde(default)]
    pub unread_idle_output: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub queued_prompt_controls: BTreeMap<String, AgentQueuedPromptControlProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<AgentActiveTurnProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_turn: Option<crate::git_observer::CompletedGitTurnActionProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveTurnProjection {
    pub prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    pub status: AgentPromptRuntimeStatus,
    pub phase: AgentTurnRuntimePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
}

impl SessionSnapshotProjection {
    pub fn from_daemon_app(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<Self, DaemonError> {
        Self::from_daemon_app_for_user(app, session_id, last_event_id, None)
    }

    pub fn from_daemon_app_for_user(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
        unread_for_user_id: Option<&str>,
    ) -> Result<Self, DaemonError> {
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| {
                app.providers()
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| app.provider_run_projection_store().get(provider_run_id))
            });
        let prompt_activity = app.prompt_activity_store();
        let prompt_activity = prompt_activity.read();
        let active_turns = app.active_turn_store().snapshot();
        let completed_git_turn_snapshots = app.completed_git_turn_snapshot_store();
        let agent_activity = agent_activity_for_session_projection(
            &session,
            |agent_id| {
                app.providers()
                    .get_run_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        app.provider_run_projection_store()
                            .get_for_agent(session.id(), agent_id)
                    })
            },
            &prompt_activity,
            &active_turns,
            unread_for_user_id,
            |agent_id| {
                completed_git_turn_snapshots.latest_projection_for_agent(session.id(), agent_id)
            },
        );
        Ok(Self {
            metadata: ProjectionMetadata::new(3, last_event_id),
            session,
            provider_run,
            agent_activity,
        })
    }
}

pub(crate) fn agent_activity_for_session_projection(
    session: &RuntimeSession,
    provider_run_for_agent: impl Fn(&str) -> Option<RuntimeProviderRun>,
    prompt_activity: &BTreeMap<String, ActivePromptState>,
    active_turns: &BTreeMap<String, ActiveTurnState>,
    unread_for_user_id: Option<&str>,
    completed_turn_for_agent: impl Fn(
        &str,
    )
        -> Option<crate::git_observer::CompletedGitTurnActionProjection>,
) -> BTreeMap<String, AgentRuntimeActivity> {
    let mut activity = BTreeMap::new();

    for agent in session.agents() {
        let prompt_state = session.prompt_states().get(agent.id());
        let active_prompt = prompt_state.and_then(|state| state.active_prompt());
        let queued_prompt_count = prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0);
        let provider_run = provider_run_for_agent(agent.id());
        let provider_turn_activity =
            active_turn_for_session_agent(active_turns, session.id(), agent.id());
        let provider_prompt_activity = provider_turn_activity
            .and_then(|turn| prompt_activity.get(&turn.provider_run_id))
            .or_else(|| {
                provider_run
                    .as_ref()
                    .and_then(|run| prompt_activity.get(run.id()))
            });
        let prompt_status = match active_prompt.map(PromptQueueItem::status) {
            Some(PromptStatus::Cancelling) => AgentPromptRuntimeStatus::Cancelling,
            Some(PromptStatus::Dispatching) => AgentPromptRuntimeStatus::Dispatching,
            Some(PromptStatus::Running) => {
                let settlement_requested = provider_turn_activity
                    .map(|state| state.settlement_requested)
                    .or_else(|| provider_prompt_activity.map(|state| state.settlement_requested))
                    .unwrap_or(false);
                if settlement_requested {
                    AgentPromptRuntimeStatus::Settling
                } else {
                    AgentPromptRuntimeStatus::Running
                }
            }
            Some(PromptStatus::Queued) => AgentPromptRuntimeStatus::Queued,
            Some(PromptStatus::Completed) | Some(PromptStatus::Cancelled) | None => {
                if provider_turn_activity.is_some_and(|state| state.settlement_requested) {
                    AgentPromptRuntimeStatus::Settling
                } else if provider_turn_activity.is_some() {
                    AgentPromptRuntimeStatus::Running
                } else if queued_prompt_count > 0 {
                    AgentPromptRuntimeStatus::Queued
                } else {
                    AgentPromptRuntimeStatus::None
                }
            }
        };
        let provider_busy = provider_turn_activity.is_some()
            && provider_run.as_ref().map_or(true, |run| {
                matches!(
                    run.state(),
                    ProviderRunState::Starting | ProviderRunState::Running
                )
            });
        let active_turn = provider_turn_activity
            .map(|turn| {
                let active_prompt_for_turn =
                    active_prompt.filter(|prompt| prompt.id() == turn.prompt_id);
                let prompt_origin = active_prompt_for_turn.map(PromptQueueItem::prompt_origin);
                let external_observed_id =
                    active_prompt_for_turn.and_then(PromptQueueItem::external_observed_id);
                active_turn_projection(
                    turn.prompt_id.clone(),
                    Some(turn.provider_run_id.clone()),
                    prompt_origin,
                    external_observed_id,
                    prompt_status.clone(),
                    AgentTurnRuntimePhase::from(&turn.phase),
                    Some(turn.started_at_ms),
                )
            })
            .or_else(|| {
                active_prompt.map(|prompt| {
                    active_turn_projection(
                        prompt.id().to_string(),
                        provider_run.as_ref().map(|run| run.id().to_string()),
                        Some(prompt.prompt_origin()),
                        prompt.external_observed_id(),
                        prompt_status.clone(),
                        AgentTurnRuntimePhase::Accepted,
                        None,
                    )
                })
            });
        let prompt_busy = agent_prompt_runtime_status_is_active_prompt(&prompt_status);
        let agent_busy =
            agent.is_processing() || agent.state() == AgentState::Working || provider_busy;
        let status = if agent.state() == AgentState::Error {
            AgentRuntimeStatus::Error
        } else if prompt_busy || agent_busy {
            AgentRuntimeStatus::Working
        } else {
            AgentRuntimeStatus::Idle
        };
        let active_prompt_count = usize::from(
            agent_prompt_runtime_status_is_active_prompt(&prompt_status) || active_turn.is_some(),
        );
        activity.insert(
            agent.id().to_string(),
            AgentRuntimeActivity {
                busy: status == AgentRuntimeStatus::Working,
                active_prompt_count,
                queued_prompt_count,
                unread_idle_output: status == AgentRuntimeStatus::Idle
                    && unread_for_user_id.is_some_and(|user_id| {
                        session.agent_has_unread_output(user_id, agent.id())
                    }),
                queued_prompt_controls: queued_prompt_controls_projection(
                    prompt_state,
                    active_turn.as_ref().and_then(|turn| turn.prompt_origin),
                ),
                status,
                prompt_status,
                active_turn,
                last_completed_turn: completed_turn_for_agent(agent.id()),
            },
        );
    }

    activity
}

fn active_turn_for_session_agent<'a>(
    active_turns: &'a BTreeMap<String, ActiveTurnState>,
    session_id: &str,
    agent_id: &str,
) -> Option<&'a ActiveTurnState> {
    active_turns
        .values()
        .filter(|turn| turn.session_id == session_id && turn.agent_id == agent_id)
        .max_by(|left, right| {
            left.started_at_ms
                .cmp(&right.started_at_ms)
                .then_with(|| left.provider_run_id.cmp(&right.provider_run_id))
        })
}

fn agent_prompt_runtime_status_is_active_prompt(status: &AgentPromptRuntimeStatus) -> bool {
    matches!(
        status,
        AgentPromptRuntimeStatus::Running
            | AgentPromptRuntimeStatus::Dispatching
            | AgentPromptRuntimeStatus::Cancelling
            | AgentPromptRuntimeStatus::Settling
    )
}

fn active_turn_projection(
    prompt_id: String,
    provider_run_id: Option<String>,
    prompt_origin: Option<PromptOrigin>,
    external_observed_id: Option<crate::history::ExternalProviderObservedId>,
    status: AgentPromptRuntimeStatus,
    phase: AgentTurnRuntimePhase,
    started_at_ms: Option<u64>,
) -> AgentActiveTurnProjection {
    let external_from_prompt_id = crate::history::parse_external_provider_observed_id(&prompt_id);
    let external = external_observed_id.or_else(|| {
        (prompt_origin != Some(PromptOrigin::Arroba))
            .then_some(external_from_prompt_id.clone())
            .flatten()
    });
    let prompt_origin = prompt_origin.or_else(|| external.as_ref().map(|_| PromptOrigin::External));
    AgentActiveTurnProjection {
        prompt_id,
        provider_run_id,
        prompt_origin,
        external_provider: external.as_ref().map(|metadata| metadata.provider.clone()),
        external_provider_session_id: external
            .as_ref()
            .map(|metadata| metadata.provider_session_id.clone()),
        external_provider_turn_id: external.map(|metadata| metadata.provider_turn_id),
        status,
        phase,
        started_at_ms,
    }
}

impl From<&ActiveTurnPhase> for AgentTurnRuntimePhase {
    fn from(value: &ActiveTurnPhase) -> Self {
        match value {
            ActiveTurnPhase::Accepted => Self::Accepted,
            ActiveTurnPhase::AwaitingFirstOutput => Self::AwaitingFirstOutput,
            ActiveTurnPhase::Streaming => Self::Streaming,
            ActiveTurnPhase::Settling => Self::Settling,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{
        AgentPromptRuntimeStatus, AgentRuntimeStatus, AgentTurnRuntimePhase,
        SessionSnapshotProjection,
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

        assert_eq!(projection.metadata.projection_version, 3);
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
        let mut provider_run =
            crate::provider::RuntimeProviderRun::from_control_capability_inference(
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
    fn session_snapshot_projection_infers_external_origin_from_active_turn_prompt_id() {
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
        let external_prompt = crate::session::PromptQueueItem::new(
            "external:codex:session-1:user-1",
            "external:codex",
            agent.id(),
            "external prompt",
            crate::session::PromptStatus::Running,
        )
        .with_prompt_origin(crate::session::PromptOrigin::External);
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
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
    fn session_snapshot_projection_blocks_steering_behind_sparse_external_active_turn() {
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
        .expect("prompt should queue behind sparse external turn");

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
        app.active_turn_store().start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "external:codex:session-1:user-1".to_string(),
                "cold-provider-run".to_string(),
            )
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
        let external_prompt = crate::session::PromptQueueItem::new(
            "external:codex:session-1:user-1",
            "external:codex",
            agent.id(),
            "external prompt",
            crate::session::PromptStatus::Running,
        )
        .with_prompt_origin(crate::session::PromptOrigin::External);
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
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
}
