use super::*;

#[cfg(test)]
pub(super) struct ManagedActivityRecordPause {
    arrived: std::sync::mpsc::SyncSender<()>,
    release: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
}

#[cfg(test)]
pub(super) struct ManagedActivityLockProbe {
    attempted: std::sync::mpsc::SyncSender<()>,
    acquired: std::sync::mpsc::SyncSender<()>,
}

impl KernelRuntimeState {
    pub(crate) fn managed_activity_change_sequence(&self) -> u64 {
        self.owned.runtime_projection_changes.sequence()
    }

    pub(crate) fn managed_activity_snapshot(&self) -> (u64, u8) {
        let _mutation = self
            .owned
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        loop {
            let sequence = self.managed_activity_change_sequence();
            let running_agent_count = self.owned.managed_running_agent_count_unlocked();
            if sequence == self.managed_activity_change_sequence() {
                return (sequence, running_agent_count);
            }
        }
    }

    pub(crate) fn ensure_managed_activity_tracking(
        &self,
        kernel_id: &str,
    ) -> Result<(), crate::error::DaemonError> {
        let _mutation = self
            .owned
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        let sequence = self.managed_activity_change_sequence();
        self.owned
            .managed_activity_transitions
            .enable_before_activity(kernel_id, sequence)?;
        self.owned.record_managed_activity_transition_unlocked();
        Ok(())
    }

    pub(crate) fn managed_activity_report_snapshot(
        &self,
    ) -> Result<(u64, super::ManagedActivityObservation), crate::error::DaemonError> {
        let _mutation = self
            .owned
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        loop {
            let sequence = self.managed_activity_change_sequence();
            let running_agent_count = self.owned.managed_running_agent_count_unlocked();
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
        let _mutation = self
            .owned
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        self.owned.managed_running_agent_count_unlocked()
    }

    #[cfg(test)]
    pub(crate) fn record_managed_activity_transition_for_test(&self) {
        self.owned.record_managed_activity_transition();
    }

    #[cfg(test)]
    pub(crate) fn clear_prompt_activity_for_managed_activity_test(&self, provider_run_id: &str) {
        let _ = self.owned.clear_prompt_activity(provider_run_id);
    }

    #[cfg(test)]
    fn pause_next_managed_activity_record_for_test(
        &self,
        arrived: std::sync::mpsc::SyncSender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) {
        *self
            .owned
            .managed_activity_before_record_pause
            .lock()
            .expect("managed activity test pause mutex poisoned") =
            Some(ManagedActivityRecordPause {
                arrived,
                release: std::sync::Arc::new(std::sync::Mutex::new(release)),
            });
    }

    #[cfg(test)]
    fn probe_next_managed_activity_lock_for_test(
        &self,
        attempted: std::sync::mpsc::SyncSender<()>,
        acquired: std::sync::mpsc::SyncSender<()>,
    ) {
        *self
            .owned
            .managed_activity_next_lock_probe
            .lock()
            .expect("managed activity test lock probe mutex poisoned") =
            Some(ManagedActivityLockProbe {
                attempted,
                acquired,
            });
    }
}

impl KernelRuntimeOwnedState {
    fn managed_running_agent_count_unlocked(&self) -> u8 {
        let active_turn_count = self.active_turns.snapshot().len();
        let sessions = self
            .session_store
            .list_non_ended_sessions_including_hidden();
        running_agent_count(active_turn_count, sessions)
    }

    pub(super) fn record_managed_activity_transition(&self) {
        let _mutation = self
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        self.record_managed_activity_transition_unlocked();
    }

    pub(super) fn begin_managed_activity_mutation(&self) -> ManagedActivityMutation<'_> {
        #[cfg(test)]
        let probe = self
            .managed_activity_next_lock_probe
            .lock()
            .expect("managed activity test lock probe mutex poisoned")
            .take();
        #[cfg(test)]
        if let Some(probe) = probe.as_ref() {
            probe
                .attempted
                .send(())
                .expect("managed activity lock attempt receiver should remain available");
        }
        let guard = self
            .managed_activity_mutation_lock
            .lock()
            .expect("managed activity mutation mutex poisoned");
        #[cfg(test)]
        if let Some(probe) = probe {
            probe
                .acquired
                .send(())
                .expect("managed activity lock acquisition receiver should remain available");
        }
        ManagedActivityMutation {
            state: self,
            _guard: guard,
        }
    }

    fn record_managed_activity_transition_unlocked(&self) {
        self.record_managed_activity_transition_at_unlocked(None);
    }

    fn record_managed_activity_transition_at_unlocked(&self, observed_at_ms: Option<u64>) {
        if !self.managed_activity_transitions.is_enabled() {
            return;
        }
        #[cfg(test)]
        let pause = self
            .managed_activity_before_record_pause
            .lock()
            .expect("managed activity test pause mutex poisoned")
            .take();
        #[cfg(test)]
        if let Some(pause) = pause {
            pause
                .arrived
                .send(())
                .expect("managed activity record arrival receiver should remain available");
            pause
                .release
                .lock()
                .expect("managed activity record release mutex poisoned")
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("managed activity record pause should be released");
        }
        if let Err(error) = self
            .managed_activity_transitions
            .record_current_transition(|| {
                (
                    self.runtime_projection_changes.sequence(),
                    self.managed_running_agent_count_unlocked(),
                    observed_at_ms.unwrap_or_else(crate::session::unix_epoch_ms),
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

pub(super) struct ManagedActivityMutation<'a> {
    state: &'a KernelRuntimeOwnedState,
    _guard: std::sync::MutexGuard<'a, ()>,
}

impl ManagedActivityMutation<'_> {
    // Persist the state sampled under the same boundary as its authoritative local mutation.
    // Consuming the guard prevents a later mutation from overtaking this capture.
    pub(super) fn record(self) {
        self.state.record_managed_activity_transition_unlocked();
    }

    pub(super) fn record_at(self, observed_at_ms: Option<u64>) {
        self.state.record_managed_activity_transition_at_unlocked(observed_at_ms);
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
    async fn concurrent_busy_idle_callers_capture_both_edges_in_mutation_order() {
        const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("kernel-mutation-order")
            .expect("managed activity tracking should activate");

        let (busy_arrived_tx, busy_arrived_rx) = std::sync::mpsc::sync_channel(1);
        let (release_busy_tx, release_busy_rx) = std::sync::mpsc::sync_channel(1);
        runtime.pause_next_managed_activity_record_for_test(busy_arrived_tx, release_busy_rx);

        let busy_runtime = runtime.clone();
        let (busy_done_tx, busy_done_rx) = std::sync::mpsc::sync_channel(1);
        let busy = std::thread::spawn(move || {
            busy_runtime.start_active_turn_with_trace_id(
                "session-1",
                "agent-1",
                "prompt-1",
                "provider-run-1",
                "trace-1",
            );
            busy_done_tx
                .send(())
                .expect("busy completion receiver should remain available");
        });
        busy_arrived_rx
            .recv_timeout(WAIT)
            .expect("busy caller should pause after its actual mutation");

        let (idle_attempted_tx, idle_attempted_rx) = std::sync::mpsc::sync_channel(1);
        let (idle_acquired_tx, idle_acquired_rx) = std::sync::mpsc::sync_channel(1);
        runtime.probe_next_managed_activity_lock_for_test(idle_attempted_tx, idle_acquired_tx);
        let idle_runtime = runtime.clone();
        let (idle_done_tx, idle_done_rx) = std::sync::mpsc::sync_channel(1);
        let idle = std::thread::spawn(move || {
            idle_runtime.clear_prompt_activity_for_managed_activity_test("provider-run-1");
            idle_done_tx
                .send(())
                .expect("idle completion receiver should remain available");
        });
        idle_attempted_rx
            .recv_timeout(WAIT)
            .expect("idle caller should reach the actual mutation lock");
        assert!(matches!(
            idle_acquired_rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(runtime
            .owned
            .active_turns
            .get("provider-run-1")
            .is_some());

        release_busy_tx
            .send(())
            .expect("paused busy capture should still be waiting");
        busy_done_rx
            .recv_timeout(WAIT)
            .expect("busy caller should finish after release");
        busy.join().expect("busy caller should finish");
        idle_acquired_rx
            .recv_timeout(WAIT)
            .expect("idle caller should acquire after busy capture");
        idle_done_rx
            .recv_timeout(WAIT)
            .expect("idle caller should finish after acquiring the boundary");
        idle.join().expect("idle caller should finish");

        let events = runtime
            .owned
            .durable_state_store
            .load_subject_events_by_kind(
                "kernel-mutation-order",
                "managed_kernel.activity.changed",
                10,
            )
            .expect("activity transitions should be readable");
        let counts = events
            .iter()
            .map(|event| event.payload["runningAgentCount"].as_u64())
            .collect::<Vec<_>>();
        assert_eq!(counts, vec![Some(0), Some(1), Some(0)]);
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
