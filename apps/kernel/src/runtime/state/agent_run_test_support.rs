//! Lets a router test end a sub-agent's turn through the output pump's own settlement.
use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;

impl KernelRuntimeState {
    /// See `AgentRunWaiters::pause_next_claim`.
    pub(crate) fn pause_next_worker_claim(&self) -> tokio::sync::oneshot::Receiver<()> {
        self.owned.agent_run_waiters.pause_next_claim()
    }

    /// Simulate a process exit at the paused handoff, before either side sees the end.
    pub(crate) fn discard_paused_worker_claim(&self) {
        self.owned.agent_run_waiters.discard_paused_claim();
    }

    /// `agent`'s admitted turn begins, arms a timer it never waits on and is blocked; returns the
    /// task id.
    pub(crate) fn block_admitted_task_for_test(&self, room: &str, agent: &str) -> String {
        use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
        let owned = &self.owned;
        let store = &owned.durable_state_store;
        let prompt = owned
            .session_store
            .get_session(room)
            .unwrap()
            .active_prompt_for_agent(agent)
            .unwrap()
            .id()
            .to_owned();
        let run = owned.provider_store.get_run_for_agent(room, agent).unwrap();
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: room.into(),
                agent: agent.into(),
                prompt: prompt.clone(),
                run: Some(run.id().into()),
                now: 1,
            })
            .unwrap();
        let task = store
            .agent_tasks(Some(room), Some(agent))
            .unwrap()
            .pop()
            .unwrap();
        let now = crate::session::unix_epoch_ms();
        let wake_id = format!("wake-blk-{agent}");
        let wake = serde_json::from_value(serde_json::json!({
            "id": wake_id, "task_id": task.task_id, "room_id": task.room_id,
            "agent_id": task.agent_id, "registration_id": format!("completion-{wake_id}"),
            "kind": "timer", "label": "wait for review", "state": "scheduled",
            "created_at_ms": now, "verified_at_ms": null, "next_due_ms": now + 3_600_000,
            "interval_ms": null, "command": [], "match_text": null, "matched_at_ms": null,
            "pid": null, "exit_code": null, "fire_count": 0, "missed_fires": 0,
            "last_fired_at_ms": null, "last_sequence": null, "last_delivery": null,
            "last_delivered_at_ms": null, "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap();
        store
            .agent_lifecycle(Operation::CreateWake {
                task: task.task_id.clone(),
                prompt: prompt.clone(),
                wake,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Block {
                task: task.task_id.clone(),
                prompt,
                reason: "Repeated invalid turn end; owner must resume or cancel".into(),
            })
            .unwrap();
        let task = store.agent_task(&task.task_id).unwrap().unwrap();
        assert_eq!(task.state, ExecutionState::Blocked);
        task.task_id
    }

    pub(crate) fn task_state_for_test(
        &self,
        task: &str,
    ) -> crate::durable_state::agent_lifecycle::ExecutionState {
        self.owned
            .durable_state_store
            .agent_task(task)
            .unwrap()
            .unwrap()
            .state
    }

    pub(crate) fn worker_brief_recorded_for_test(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> bool {
        let merge_key = format!("prompt:{prompt}");
        self.owned
            .operational_history_store
            .load_session_history_entries(room, Some(agent))
            .unwrap()
            .iter()
            .any(|entry| {
                entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                    && entry.merge_key.as_deref() == Some(merge_key.as_str())
            })
    }

    pub(crate) fn record_observed_recovery_envelope_for_test(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) {
        self.owned.append_history_entries(
            room,
            vec![crate::history::SessionHistoryEntry::external_provider_observed(
                room,
                None,
                agent,
                crate::history::SessionHistoryEntryKind::UserPrompt,
                format!("[Chariox recovery operation chariox-recovery:{prompt}:1] Continue the active task."),
                "claude",
                "restarted-session",
                Some("recovery-envelope".into()),
                None,
            )],
        );
    }

    /// The agent's running turn answers `answer` and settles, as its provider's output does.
    pub(crate) async fn end_agent_turn_for_test(
        &self,
        room: &str,
        agent: &str,
        answer: &str,
    ) -> Result<(), DaemonError> {
        let run = self
            .owned
            .provider_store
            .get_run_for_agent(room, agent)
            .expect("the agent has a provider run")
            .id()
            .to_string();
        self.apply_owned_structured_output_batch(
            room,
            &run,
            Vec::new(),
            ProviderPromptSignalBatch {
                chunks: vec![ProviderPromptChunk {
                    kind: TerminalOutputKind::ProviderOutput,
                    merge_key: None,
                    bytes: answer.as_bytes().to_vec(),
                }],
                ..Default::default()
            },
        )
        .await?;
        self.flush_provider_history(&run).await?;
        self.settle_owned_provider_prompt(room, &run, true, false, true)
            .await
            .map(|_| ())
    }
}
