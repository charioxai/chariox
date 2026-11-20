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
