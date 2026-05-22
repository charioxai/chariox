use super::SessionService;
use crate::agent::{AgentInstance, GridPosition};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::session::{
    unix_epoch_ms, CreateSessionRequest, PromptSubmissionOutcome, SchedulerState,
    SessionAgentDefaults, SessionStatus, WorkflowCompletionSnapshot, WorkflowHandoffPayload,
    WorkflowNodeRunStatus, WorkflowQueuedPromptSource, WorkflowRunStatus, WorkflowWatchdogPolicy,
    WorktreeIsolationMode, DEFAULT_LOCAL_USER_ID,
};
use std::collections::BTreeMap;

fn test_config() -> DaemonConfig {
    DaemonConfig::for_tests()
}

fn seed_agents(service: &mut SessionService, session_id: &str, agent_ids: &[&str]) {
    let session = service
        .store
        .get_mut(session_id)
        .expect("session should exist for test seeding");
    let agents = agent_ids
        .iter()
        .enumerate()
        .map(|(index, agent_id)| {
            AgentInstance::new(
                agent_id.to_string(),
                format!("ref-{agent_id}"),
                session_id.to_string(),
                None,
                "dev-stub",
                Some("default".to_string()),
                None,
                None,
                GridPosition::new(0, index as u32, 1, 1),
            )
        })
        .collect::<Vec<_>>();
    session.set_agents(agents);
}

mod session_lifecycle;
mod workflow_definitions;
mod workflow_dispatch;
mod workflow_runs;
