use std::collections::HashSet;

use crate::agent::{AgentInstance, AgentState};
use crate::local::{WaitingRoomPublicItemActivitySummary, WaitingRoomSessionActivitySummary};
use crate::session::{RuntimeSession, WorkflowRunStatus};

pub(crate) fn waiting_room_agent_activity_summary(
    session: &RuntimeSession,
    agent: &AgentInstance,
    caller_user_id: &str,
) -> WaitingRoomPublicItemActivitySummary {
    let active_prompt_count = usize::from(session.active_prompt_for_agent(agent.id()).is_some());
    let queued_prompt_count = session
        .queued_prompts_for_agent(agent.id())
        .map(|queued| queued.len())
        .unwrap_or(0);
    let error = agent.state() == AgentState::Error;
    WaitingRoomPublicItemActivitySummary {
        working: agent.state() == AgentState::Working
            || agent.is_processing()
            || active_prompt_count > 0,
        active_prompt_count,
        queued_prompt_count,
        error,
        unread_idle_output: active_prompt_count == 0
            && !agent.is_processing()
            && agent.state() != AgentState::Working
            && session.agent_has_unread_output(caller_user_id, agent.id()),
    }
}

pub(crate) fn waiting_room_workflow_activity_summary(
    session: &RuntimeSession,
    workflow_id: &str,
) -> WaitingRoomPublicItemActivitySummary {
    let working = session.workflow_runs().iter().any(|run| {
        run.workflow_id() == workflow_id
            && matches!(
                run.status(),
                WorkflowRunStatus::Created
                    | WorkflowRunStatus::Running
                    | WorkflowRunStatus::Waiting
                    | WorkflowRunStatus::Completing
            )
    });
    let error = session.workflow_runs().iter().any(|run| {
        run.workflow_id() == workflow_id && matches!(run.status(), WorkflowRunStatus::Failed)
    });
    WaitingRoomPublicItemActivitySummary {
        working,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error,
        unread_idle_output: false,
    }
}

pub(crate) fn waiting_room_session_activity_summary(
    session: &RuntimeSession,
    caller_user_id: &str,
) -> WaitingRoomSessionActivitySummary {
    let active_prompt_agent_ids: HashSet<&str> = session
        .prompt_states()
        .iter()
        .filter(|(_, state)| state.active_prompt().is_some())
        .map(|(agent_id, _)| agent_id.as_str())
        .collect();
    let active_prompt_count = if active_prompt_agent_ids.is_empty() && session.has_active_prompt() {
        1
    } else {
        active_prompt_agent_ids.len()
    };
    let queued_prompt_count = if session.prompt_states().is_empty() {
        session.queued_prompts().len()
    } else {
        session
            .prompt_states()
            .values()
            .map(|state| state.queued_prompts().len())
            .sum()
    };
    let mut working_agent_count = session
        .agents()
        .iter()
        .filter(|agent| {
            agent.state() == AgentState::Working
                || agent.is_processing()
                || active_prompt_agent_ids.contains(agent.id())
        })
        .count();
    if working_agent_count == 0 && active_prompt_count > 0 {
        working_agent_count = 1;
    }
    let remote_agent_count = session
        .agents()
        .iter()
        .filter(|agent| agent.remote_execution().is_some())
        .count();
    let missing_worker_provider_run_count = session
        .agents()
        .iter()
        .filter(|agent| {
            let Some(remote) = agent.remote_execution() else {
                return false;
            };
            let active_worker_run_missing = remote
                .active_worker_provider_run_id
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty();
            active_worker_run_missing
                && (agent.state() == AgentState::Working
                    || agent.is_processing()
                    || active_prompt_agent_ids.contains(agent.id()))
        })
        .count();
    WaitingRoomSessionActivitySummary {
        agent_count: session.agents().len(),
        working_agent_count,
        active_prompt_count,
        queued_prompt_count,
        error_agent_count: session
            .agents()
            .iter()
            .filter(|agent| agent.state() == AgentState::Error)
            .count(),
        remote_agent_count,
        missing_worker_provider_run_count,
        unread_idle_agent_count: session
            .agents()
            .iter()
            .filter(|agent| {
                !active_prompt_agent_ids.contains(agent.id())
                    && agent.state() != AgentState::Working
                    && !agent.is_processing()
                    && session.agent_has_unread_output(caller_user_id, agent.id())
            })
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::{AgentInstance, AgentState, GridPosition, RemoteAgentBinding};
    use crate::runtime::waiting_room_activity::{
        waiting_room_agent_activity_summary, waiting_room_session_activity_summary,
        waiting_room_workflow_activity_summary,
    };
    use crate::session::{RuntimeSession, WorkflowRun, WorkflowRunStatus};

    fn session_with_agents(agents: Vec<AgentInstance>) -> RuntimeSession {
        let mut session =
            RuntimeSession::new("session", None, "/repo", "/repo", "machine", "daemon");
        session.set_agents(agents);
        session
    }

    fn agent(id: &str, state: AgentState, processing: bool) -> AgentInstance {
        let mut agent = AgentInstance::new(
            id,
            id,
            "session",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_state(state);
        agent.set_processing(processing);
        agent
    }

    fn remote_agent(
        id: &str,
        state: AgentState,
        processing: bool,
        run_id: Option<&str>,
    ) -> AgentInstance {
        let mut agent = agent(id, state, processing);
        agent.set_remote_execution(Some(RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: format!("lease-{id}"),
            leased_agent_id: format!("leased-{id}"),
            active_worker_provider_run_id: run_id.map(str::to_string),
            relay_url: None,
            relay_token: None,
        }));
        agent
    }

    #[test]
    fn agent_activity_tracks_working_processing_and_error_state() {
        let idle = agent("idle", AgentState::Idle, false);
        let working = agent("working", AgentState::Working, false);
        let processing = agent("processing", AgentState::Idle, true);
        let error = agent("error", AgentState::Error, false);
        let session = session_with_agents(vec![
            idle.clone(),
            working.clone(),
            processing.clone(),
            error.clone(),
        ]);

        assert!(
            !waiting_room_agent_activity_summary(
                &session,
                &idle,
                crate::session::DEFAULT_LOCAL_USER_ID
            )
            .working
        );
        assert!(
            waiting_room_agent_activity_summary(
                &session,
                &working,
                crate::session::DEFAULT_LOCAL_USER_ID
            )
            .working
        );
        assert!(
            waiting_room_agent_activity_summary(
                &session,
                &processing,
                crate::session::DEFAULT_LOCAL_USER_ID
            )
            .working
        );
        assert!(
            waiting_room_agent_activity_summary(
                &session,
                &error,
                crate::session::DEFAULT_LOCAL_USER_ID
            )
            .error
        );
    }

    #[test]
    fn workflow_activity_tracks_active_and_failed_runs() {
        let mut active_session = session_with_agents(Vec::new());
        active_session.create_workflow_run(WorkflowRun::new(
            "run-1",
            "workflow",
            "endpoint",
            "node",
            None,
            None,
            Vec::new(),
            Vec::new(),
        ));
        assert!(waiting_room_workflow_activity_summary(&active_session, "workflow").working);
        assert!(!waiting_room_workflow_activity_summary(&active_session, "workflow").error);

        let mut failed_session = session_with_agents(Vec::new());
        let mut failed = WorkflowRun::new(
            "run-2",
            "workflow",
            "endpoint",
            "node",
            None,
            None,
            Vec::new(),
            Vec::new(),
        );
        failed.set_status(WorkflowRunStatus::Failed);
        failed_session.create_workflow_run(failed);
        assert!(!waiting_room_workflow_activity_summary(&failed_session, "workflow").working);
        assert!(waiting_room_workflow_activity_summary(&failed_session, "workflow").error);
    }

    #[test]
    fn session_activity_summarizes_agent_counts() {
        let session = session_with_agents(vec![
            agent("working", AgentState::Working, false),
            agent("processing", AgentState::Idle, true),
            agent("error", AgentState::Error, false),
        ]);

        let summary =
            waiting_room_session_activity_summary(&session, crate::session::DEFAULT_LOCAL_USER_ID);
        assert_eq!(summary.agent_count, 3);
        assert_eq!(summary.working_agent_count, 2);
        assert_eq!(summary.error_agent_count, 1);
        assert_eq!(summary.active_prompt_count, 0);
        assert_eq!(summary.queued_prompt_count, 0);
    }

    #[test]
    fn session_activity_summarizes_remote_worker_run_blockers() {
        let session = session_with_agents(vec![
            remote_agent("remote-working-missing", AgentState::Working, false, None),
            remote_agent("remote-processing-empty", AgentState::Idle, true, Some("")),
            remote_agent("remote-idle-missing", AgentState::Idle, false, None),
            remote_agent(
                "remote-working-ready",
                AgentState::Working,
                false,
                Some("worker-run"),
            ),
            agent("local-working", AgentState::Working, false),
        ]);

        let summary =
            waiting_room_session_activity_summary(&session, crate::session::DEFAULT_LOCAL_USER_ID);
        assert_eq!(summary.remote_agent_count, 4);
        assert_eq!(summary.missing_worker_provider_run_count, 2);
    }

    #[test]
    fn idle_provider_output_projects_unread_until_acknowledged() {
        let mut session = session_with_agents(vec![agent("agent-1", AgentState::Idle, false)]);
        assert!(session.note_agent_output_sequence("agent-1", 41));

        let agent = session
            .agents()
            .iter()
            .find(|agent| agent.id() == "agent-1")
            .expect("agent exists");
        let summary = waiting_room_agent_activity_summary(
            &session,
            agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert!(summary.unread_idle_output);
        assert_eq!(
            waiting_room_session_activity_summary(&session, crate::session::DEFAULT_LOCAL_USER_ID)
                .unread_idle_agent_count,
            1
        );

        assert!(
            session.acknowledge_agent_output_seen(crate::session::DEFAULT_LOCAL_USER_ID, "agent-1")
        );
        let agent = session
            .agents()
            .iter()
            .find(|agent| agent.id() == "agent-1")
            .expect("agent exists");
        assert!(
            !waiting_room_agent_activity_summary(
                &session,
                agent,
                crate::session::DEFAULT_LOCAL_USER_ID
            )
            .unread_idle_output
        );
    }
}
