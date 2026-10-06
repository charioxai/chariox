//! MP-08 / MP-10 / MP-11: accepted worker counters persist under the home prompt before settlement.
use super::RemoteLeaseRuntime;
use crate::{
    error::DaemonError,
    history::{HistoryEventKind, HistoryEventRole, HistoryEventTurnContext},
    usage_accounting::report::{self, TurnUsage},
};
impl RemoteLeaseRuntime<'_> {
    pub(super) fn persist_home_turn_usage(
        &self,
        session: &str,
        agent_id: &str,
        prompt: &str,
        run_id: &str,
        counters: Option<crate::provider::ProviderRunTokenUsage>,
    ) -> Result<(), DaemonError> {
        let previous = report::latest(&self.app.operational_history, session, prompt, run_id)?;
        let agent = self.app.agents.get_agent(agent_id)?;
        let mut turn = TurnUsage {
            session_id: session.into(),
            agent_id: agent_id.into(),
            parent_agent_id: agent.controlled_by_metaagent_id().map(str::to_string),
            prompt_id: prompt.into(),
            provider_run_id: run_id.into(),
            provider: agent.provider().into(),
            model: agent.model().unwrap_or("unknown").into(),
            completed: true,
            usage: counters
                .map(|u| u.turn_accounting)
                .unwrap_or_else(|| previous.as_ref().and_then(|t| t.usage)),
            provider_counters: counters
                .map(|u| u.accounting)
                .unwrap_or_else(|| previous.as_ref().and_then(|t| t.provider_counters)),
            api_equivalent_nanodollars: None,
            price_table_version: 0,
            price_table_date: String::new(),
        };
        report::stamp(&mut turn);
        if previous.as_ref() == Some(&turn) {
            return Ok(());
        }
        self.app.operational_history.append_operational_event(
            HistoryEventKind::ProviderStatus,
            Some(HistoryEventRole::System),
            None,
            std::collections::BTreeMap::from([(
                report::METADATA_KEY.into(),
                serde_json::json!(turn),
            )]),
            HistoryEventTurnContext {
                session_id: Some(session.into()),
                agent_id: Some(agent_id.into()),
                prompt_id: Some(prompt.into()),
                provider_run_id: Some(run_id.into()),
                provider: Some(turn.provider.clone()),
                model: Some(turn.model.clone()),
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
