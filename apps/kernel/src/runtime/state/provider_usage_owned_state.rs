//! MP-08 / MP-10 / MP-11: provider usage is attributed before prompt settlement.
use super::*;
use crate::usage_accounting::report::{self, TurnUsage};
impl KernelRuntimeOwnedState {
    pub(super) fn record_provider_usage(
        &self,
        session_id: &str,
        provider_run_id: &str,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        if batch.resolved_usage.is_none()
            && !batch.prompt_completed
            && batch.terminal_failure.is_none()
        {
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
        let previous = report::latest(
            &self.operational_history_store,
            session_id,
            prompt.id(),
            provider_run_id,
        )?;
        let counters = batch.resolved_usage;
        let mut turn = TurnUsage {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            parent_agent_id: self
                .agent_store
                .get_agent(agent_id)?
                .controlled_by_metaagent_id()
                .map(str::to_string),
            prompt_id: prompt.id().into(),
            provider_run_id: provider_run_id.into(),
            provider: run.provider().into(),
            model: batch
                .resolved_model
                .clone()
                .unwrap_or_else(|| run.model().into()),
            completed: batch.prompt_completed
                || batch.terminal_failure.is_some()
                || previous.as_ref().is_some_and(|t| t.completed),
            usage: match counters {
                Some(u) => u.turn_accounting,
                None => previous.as_ref().and_then(|t| t.usage),
            },
            provider_counters: match counters {
                Some(u) => u.accounting,
                None => previous.as_ref().and_then(|t| t.provider_counters),
            },
            api_equivalent_nanodollars: None,
            price_table_version: 0,
            price_table_date: String::new(),
        };
        report::stamp(&mut turn);
        if previous.as_ref() == Some(&turn) {
            return Ok(());
        }
        let metadata = std::collections::BTreeMap::from([(
            report::METADATA_KEY.into(),
            serde_json::json!(turn),
        )]);
        self.operational_history_store.append_operational_event(
            crate::history::HistoryEventKind::ProviderStatus,
            Some(crate::history::HistoryEventRole::System),
            None,
            metadata,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session_id.into()),
                agent_id: Some(agent_id.into()),
                prompt_id: Some(prompt.id().into()),
                provider_run_id: Some(provider_run_id.into()),
                provider: Some(run.provider().into()),
                model: Some(run.model().into()),
                provider_session_id: run.provider_session_id().map(str::to_string),
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
