use super::*;

impl KernelRuntimeState {
    pub(crate) fn managed_activity_change_sequence(&self) -> u64 {
        self.owned.runtime_projection_changes.sequence()
    }

    pub(crate) async fn wait_for_managed_activity_transition_after(
        &self,
        mut sequence: u64,
        running_agent_count: u8,
    ) -> (u64, u8) {
        loop {
            let latest_sequence = self.managed_activity_change_sequence();
            if latest_sequence != sequence {
                sequence = latest_sequence;
                let latest_count = self.managed_running_agent_count();
                if latest_count != running_agent_count {
                    return (sequence, latest_count);
                }
            }
            self.owned
                .runtime_projection_changes
                .wait_for_change_after(sequence)
                .await;
        }
    }

    pub(crate) fn managed_running_agent_count(&self) -> u8 {
        let active_turn_count = self.owned.active_turns.snapshot().len();
        let sessions = self
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .into_iter()
            .map(|mut session| {
                self.owned.project_session_runtime_view(&mut session);
                session
            });
        running_agent_count(active_turn_count, sessions)
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
        RuntimeInteractionLevel, RuntimeSession,
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
        let sequence = runtime.managed_activity_change_sequence();
        assert_eq!(runtime.managed_running_agent_count(), 0);

        runtime.owned.runtime_projection_changes.record_change();
        let wait = runtime.wait_for_managed_activity_transition_after(sequence, 0);
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
        runtime.owned.runtime_projection_changes.record_change();
        let (_, running_agent_count) =
            tokio::time::timeout(std::time::Duration::from_secs(1), wait)
                .await
                .expect("real activity transition should wake");
        assert_eq!(running_agent_count, 1);
    }
}
