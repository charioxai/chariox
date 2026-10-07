//! MP-08 / MP-10 / MP-11: accepted worker counters persist under the home prompt before settlement.
use super::RemoteLeaseRuntime;
use crate::{
    error::DaemonError,
    history::{HistoryEventKind, HistoryEventRole, HistoryEventTurnContext},
    usage_accounting::report::{self, TurnUsage},
};
impl RemoteLeaseRuntime<'_> {
    pub(super) fn bind_worker_turn_usage(
        &self,
        leased: &crate::execution_lease::LeasedAgent,
        run_id: &str,
        completions: &[crate::transport::relay_peer::RelayProjectedCompletion],
        active_worker_prompt_id: Option<&str>,
        run: &mut crate::provider::RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let home_prompt = completions
            .first()
            .map(|c| c.home_prompt_id.as_deref())
            .unwrap_or(leased.active_home_prompt_id.as_deref());
        let same_prompt = completions
            .iter()
            .all(|c| c.home_prompt_id.as_deref() == home_prompt);
        let worker_prompt_id = if !same_prompt {
            None
        } else if let Some(home_prompt) = home_prompt {
            self.app
                .worker_prompt_receipts
                .get(&leased.id, home_prompt)
                .filter(|r| {
                    r.receipt.execution_lease_id == leased.lease_id
                        && r.receipt.worker_provider_run_id.as_deref() == Some(run_id)
                })
                .and_then(|r| r.worker_prompt_id.clone())
        } else {
            // A worker-native turn has no home admission receipt. Its active or
            // settled worker prompt remains the accounting identity.
            active_worker_prompt_id.map(str::to_string).or_else(|| {
                self.app
                    .completed_git_turn_snapshot_store()
                    .latest_projection_for_agent(
                        &leased.backing_session_id,
                        &leased.backing_agent_id,
                    )
                    .filter(|turn| turn.provider_run_id == run_id)
                    .map(|turn| turn.prompt_id)
            })
        };
        let turn = worker_prompt_id
            .as_deref()
            .map(|prompt| {
                report::latest(
                    &self.app.operational_history,
                    &leased.backing_session_id,
                    prompt,
                    run_id,
                )
            })
            .transpose()?
            .flatten()
            .filter(|turn| turn.agent_id == leased.backing_agent_id);
        let mut counters = run.usage();
        counters.turn_accounting = turn.as_ref().and_then(|turn| turn.usage);
        counters.accounting = turn.as_ref().and_then(|turn| turn.provider_counters);
        run.set_usage(counters);
        Ok(())
    }

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
