//! Lets a router test end a sub-agent's turn through the output pump's own settlement.
use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch};
use crate::terminal::TerminalOutputKind;

impl KernelRuntimeState {
    /// See `AgentRunWaiters::pause_next_claim`.
    pub(crate) fn pause_next_worker_claim(&self) -> tokio::sync::oneshot::Receiver<()> {
        self.owned.agent_run_waiters.pause_next_claim()
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
