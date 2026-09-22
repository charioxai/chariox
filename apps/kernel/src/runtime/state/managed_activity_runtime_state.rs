use super::*;

impl KernelRuntimeState {
    pub(crate) fn managed_activity_change_sequence(&self) -> u64 {
        self.owned.runtime_projection_changes.sequence()
    }

    pub(crate) fn managed_activity_snapshot(&self) -> (u64, u8) {
        loop {
            let sequence = self.managed_activity_change_sequence();
            let running_agent_count = self.managed_running_agent_count();
            if sequence == self.managed_activity_change_sequence() {
                return (sequence, running_agent_count);
            }
        }
    }

    pub(crate) fn ensure_managed_activity_tracking(
        &self,
        kernel_id: &str,
    ) -> Result<(), crate::error::DaemonError> {
        let sequence = self.managed_activity_change_sequence();
        self.owned
            .managed_activity_transitions
            .enable_before_activity(kernel_id, sequence)?;
        self.owned.record_managed_activity_transition();
        Ok(())
    }

    pub(crate) fn managed_activity_report_snapshot(
        &self,
    ) -> Result<(u64, super::ManagedActivityObservation), crate::error::DaemonError> {
        loop {
            let (sequence, running_agent_count) = self.managed_activity_snapshot();
            match self
                .owned
                .managed_activity_transitions
                .current_observation(running_agent_count)
            {
                Ok(observation) if sequence == self.managed_activity_change_sequence() => {
                    return Ok((sequence, observation));
                }
                Ok(_) => {}
                Err(_) if sequence != self.managed_activity_change_sequence() => {}
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) async fn wait_for_managed_activity_transition_after(
        &self,
        mut sequence: u64,
        observation: super::ManagedActivityObservation,
    ) -> Result<(u64, super::ManagedActivityObservation), crate::error::DaemonError> {
        loop {
            let latest_sequence = self.managed_activity_change_sequence();
            if latest_sequence != sequence {
                let (latest_sequence, latest_observation) =
                    self.managed_activity_report_snapshot()?;
                sequence = latest_sequence;
                if latest_observation != observation {
                    return Ok((sequence, latest_observation));
                }
            }
            self.owned
                .runtime_projection_changes
                .wait_for_change_after(sequence)
                .await;
        }
    }

    pub(crate) fn managed_running_agent_count(&self) -> u8 {
        self.owned.managed_running_agent_count()
    }

    #[cfg(test)]
    pub(crate) fn record_managed_activity_transition_for_test(&self) {
        self.owned.record_managed_activity_transition();
    }

    #[cfg(test)]
    pub(crate) fn clear_prompt_activity_for_managed_activity_test(&self, provider_run_id: &str) {
        let _ = self.owned.clear_prompt_activity(provider_run_id);
    }
}

impl KernelRuntimeOwnedState {
    fn managed_running_agent_count(&self) -> u8 {
        let active_turn_count = self.active_turns.snapshot().len();
        let sessions = self
            .session_store
            .list_non_ended_sessions_including_hidden()
            .into_iter()
            .map(|mut session| {
                self.project_session_runtime_view(&mut session);
                session
            });
        running_agent_count(active_turn_count, sessions)
    }

    pub(super) fn record_managed_activity_transition(&self) {
        if !self.managed_activity_transitions.is_enabled() {
            return;
        }
        if let Err(error) = self
            .managed_activity_transitions
            .record_current_transition(|| {
                (
                    self.runtime_projection_changes.sequence(),
                    self.managed_running_agent_count(),
                    crate::session::unix_epoch_ms(),
                )
            })
        {
            crate::logging::error_with_fields(
                "managed_kernel.activity",
                "managed activity transition could not be persisted",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }
}

fn running_agent_count(
    active_turn_count: usize,
    sessions: impl IntoIterator<Item = crate::session::RuntimeSession>,
) -> u8 {
    if active_turn_count > 0
        || sessions.into_iter().any(|session| {
            session.has_any_prompt_work()
                || !session.active_interactions().is_empty()
                || session.has_active_session_task()
                || session.has_pending_session_task()
        })
    {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::running_agent_count;
    use crate::config::DaemonConfig;
    use crate::runtime::router::CommandRouter;
    use crate::session::{
        PromptQueueItem, PromptStatus, RuntimeInteraction, RuntimeInteractionKind,
        RuntimeInteractionLevel, RuntimeSession, WorkflowRun, WorkflowRunStatus,
    };
    use crate::DaemonApp;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn session() -> RuntimeSession {
        RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        )
    }

    #[test]
    fn managed_activity_counts_active_and_queued_prompt_work() {
        let mut active = session();
        active.mirror_agent_prompt_state(
            "agent-1",
            Some(PromptQueueItem::new(
                "prompt-1",
                "attachment-1",
                "agent-1",
                "work",
                PromptStatus::Running,
            )),
            VecDeque::new(),
        );
        assert_eq!(running_agent_count(0, [active]), 1);

        let mut queued = session();
        queued.mirror_agent_prompt_state(
            "agent-1",
            None,
            VecDeque::from([PromptQueueItem::new(
                "prompt-2",
                "attachment-1",
                "agent-1",
                "later",
                PromptStatus::Queued,
            )]),
        );
        assert_eq!(running_agent_count(0, [queued]), 1);
    }

    #[test]
    fn managed_activity_counts_unresolved_interactions() {
        let mut session = session();
        session.add_active_interaction(RuntimeInteraction::new(
            "interaction-1",
            "agent-1",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            None,
            "Approve?",
            Vec::new(),
            None,
            None,
            None,
        ));
        assert_eq!(running_agent_count(0, [session]), 1);
    }

    #[test]
    fn managed_activity_counts_active_and_queued_session_tasks() {
        let mut active = session();
        active.start_or_update_metaagent_task("agent-1", "active task");
        assert_eq!(running_agent_count(0, [active]), 1);

        let mut queued = session();
        queued.enqueue_metaagent_task(crate::session::QueuedMetaagentTask::new(
            "task-1",
            "agent-1",
            "attachment-1",
            "queued task",
            Vec::new(),
        ));
        assert_eq!(running_agent_count(0, [queued]), 1);
    }

    #[test]
    fn active_turn_blocks_zero_until_prompt_settlement_finishes() {
        assert_eq!(running_agent_count(1, [session()]), 1);
        assert_eq!(running_agent_count(0, [session()]), 0);
    }

    #[tokio::test]
    async fn same_count_projection_churn_does_not_interrupt_activity_wait() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("kernel-projection-churn")
            .expect("managed activity tracking should activate");
        let (sequence, observation) = runtime
            .managed_activity_report_snapshot()
            .expect("initial activity should be durable");
        assert_eq!(observation.running_agent_count, 0);

        runtime.owned.runtime_projection_changes.record_change();
        let wait = runtime.wait_for_managed_activity_transition_after(sequence, observation);
        tokio::pin!(wait);
        tokio::select! {
            biased;
            transition = &mut wait => panic!("same-count churn returned {transition:?}"),
            _ = tokio::task::yield_now() => {}
        }

        runtime
            .owned
            .active_turns
            .start(crate::app::ActiveTurnState::new(
                "session-1".to_string(),
                "agent-1".to_string(),
                "prompt-1".to_string(),
                "provider-run-1".to_string(),
            ));
        runtime.record_managed_activity_transition_for_test();
        runtime.owned.runtime_projection_changes.record_change();
        let (_, latest) = tokio::time::timeout(std::time::Duration::from_secs(1), wait)
            .await
            .expect("real activity transition should wake")
            .expect("activity transition should remain readable");
        assert_eq!(latest.running_agent_count, 1);
    }

    #[tokio::test]
    async fn rapid_busy_idle_cycle_wakes_for_later_idle_timestamp() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("kernel-rapid-wait")
            .expect("managed activity tracking should activate");
        let (sequence, initial_idle) = runtime
            .managed_activity_report_snapshot()
            .expect("initial idle activity should be durable");

        std::thread::sleep(std::time::Duration::from_millis(2));
        runtime.start_active_turn_with_trace_id(
            "session-1",
            "agent-1",
            "prompt-1",
            "provider-run-1",
            "trace-1",
        );
        runtime.record_managed_activity_transition_for_test();
        runtime.owned.runtime_projection_changes.record_change();
        std::thread::sleep(std::time::Duration::from_millis(2));
        runtime.clear_prompt_activity_for_managed_activity_test("provider-run-1");

        let (_, later_idle) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runtime.wait_for_managed_activity_transition_after(sequence, initial_idle),
        )
        .await
        .expect("completed busy-idle cycle should wake")
        .expect("later idle observation should remain readable");
        assert_eq!(later_idle.running_agent_count, 0);
        assert!(later_idle.changed_at_ms > initial_idle.changed_at_ms);
    }

    #[tokio::test]
    async fn workflow_completion_without_active_prompt_persists_idle_transition() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("kernel-workflow-completion")
            .expect("managed activity tracking should activate before work");

        let mut session = session();
        let mut workflow_run = WorkflowRun::new(
            "run-1",
            "workflow-1",
            "endpoint-1",
            "node-1",
            None,
            None,
            Vec::new(),
            Vec::new(),
        );
        workflow_run.set_status(WorkflowRunStatus::Running);
        session.create_workflow_run(workflow_run);
        assert!(!session.has_any_prompt_work());
        runtime.owned.session_store.restore_session(session);
        runtime.record_managed_activity_transition_for_test();
        let (_, busy) = runtime
            .managed_activity_report_snapshot()
            .expect("busy workflow transition should be durable");
        assert_eq!(busy.running_agent_count, 1);

        let mut session = runtime
            .owned
            .session_store
            .get_session("session-1")
            .expect("workflow session should exist");
        session
            .workflow_run_mut("run-1")
            .expect("workflow run should exist")
            .set_status(WorkflowRunStatus::Completed);
        assert!(!session.has_any_prompt_work());
        runtime.owned.session_store.restore_session(session);
        runtime
            .owned
            .persist_workflow_runtime_session("session-1", "workflow_test_completed")
            .expect("workflow completion should persist");

        let (_, idle) = runtime
            .managed_activity_report_snapshot()
            .expect("idle workflow transition should be durable");
        assert_eq!(idle.running_agent_count, 0);
        assert!(idle.changed_at_ms >= busy.changed_at_ms);
    }
}
