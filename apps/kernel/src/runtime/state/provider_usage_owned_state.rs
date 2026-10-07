//! MP-08 / MP-10 / MP-11: provider usage is attributed before prompt settlement.
use super::*;
use crate::usage_accounting::record;
impl KernelRuntimeOwnedState {
    pub(super) fn record_provider_usage(
        &self,
        session_id: &str,
        provider_run_id: &str,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        if !record::should_record(batch) {
            return Ok(());
        }
        let run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = run.agent_instance_id() else {
            return Ok(());
        };
        let session = self.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(());
        };
        record::provider_batch(
            &self.operational_history_store,
            &run,
            &self.agent_store.get_agent(agent_id)?,
            &prompt,
            batch,
        )
    }
}
