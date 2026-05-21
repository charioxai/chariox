use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ProjectionMetadata;
use crate::agent::AgentState;
use crate::app::{ActivePromptState, ActiveTurnPhase, ActiveTurnState, DaemonApp};
use crate::error::DaemonError;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::{PromptQueueItem, PromptStatus, RuntimeSession};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<AgentActiveTurnProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveTurnProjection {
    pub prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
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
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| app.providers().get_run(provider_run_id).ok());
        let prompt_activity = app.prompt_activity_store();
        let prompt_activity = prompt_activity.read();
        let active_turns = app.active_turn_store().snapshot();
        let agent_activity = agent_activity_for_session_projection(
            &session,
            |agent_id| app.providers().get_run_for_agent(session.id(), agent_id),
            &prompt_activity,
            &active_turns,
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
) -> BTreeMap<String, AgentRuntimeActivity> {
    let mut activity = BTreeMap::new();

    for agent in session.agents() {
        let prompt_state = session.prompt_states().get(agent.id());
        let active_prompt = prompt_state.and_then(|state| state.active_prompt());
        let queued_prompt_count = prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0);
        let provider_run = provider_run_for_agent(agent.id());
        let provider_prompt_activity = provider_run
            .as_ref()
            .and_then(|run| prompt_activity.get(run.id()));
        let provider_turn_activity = provider_run.as_ref().and_then(|run| {
            active_turns.get(run.id()).filter(|turn| {
                turn.session_id == session.id()
                    && turn.agent_id == agent.id()
                    && turn.provider_run_id == run.id()
            })
        });
        let prompt_status = match active_prompt.map(PromptQueueItem::status) {
            Some(PromptStatus::Cancelling) => AgentPromptRuntimeStatus::Cancelling,
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
        let provider_busy = provider_run.as_ref().is_some_and(|run| {
            matches!(
                run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            ) && provider_turn_activity.is_some()
        });
        let active_turn = provider_turn_activity
            .map(|turn| AgentActiveTurnProjection {
                prompt_id: turn.prompt_id.clone(),
                provider_run_id: Some(turn.provider_run_id.clone()),
                status: prompt_status.clone(),
                phase: AgentTurnRuntimePhase::from(&turn.phase),
                started_at_ms: Some(turn.started_at_ms),
            })
            .or_else(|| {
                active_prompt.map(|prompt| AgentActiveTurnProjection {
                    prompt_id: prompt.id().to_string(),
                    provider_run_id: provider_run.as_ref().map(|run| run.id().to_string()),
                    status: prompt_status.clone(),
                    phase: AgentTurnRuntimePhase::Accepted,
                    started_at_ms: None,
                })
            });
        let prompt_busy = !matches!(prompt_status, AgentPromptRuntimeStatus::None);
        let agent_busy =
            agent.is_processing() || agent.state() == AgentState::Working || provider_busy;
        let status = if agent.state() == AgentState::Error {
            AgentRuntimeStatus::Error
        } else if prompt_busy || agent_busy {
            AgentRuntimeStatus::Working
        } else {
            AgentRuntimeStatus::Idle
        };
        activity.insert(
            agent.id().to_string(),
            AgentRuntimeActivity {
                busy: status == AgentRuntimeStatus::Working,
                status,
                prompt_status,
                active_turn,
            },
        );
    }

    activity
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
    use crate::runtime::projection::test_support::{
        attach_cli, launch_dev_stub_provider, submit_prompt,
    };
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn session_snapshot_projection_includes_metadata_and_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");

        assert_eq!(projection.metadata.projection_version, 3);
        assert_eq!(projection.metadata.last_event_id, 42);
        assert_eq!(projection.session.id(), session.id());
        assert_eq!(projection.session.agents().len(), 1);
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
        assert!(active_turn.started_at_ms.is_some());
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
