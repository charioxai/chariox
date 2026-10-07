//! MP-08 / MP-10 / MP-11: one accounting writer for owned and leased compatibility pumps.
use super::report::{self, TurnUsage};
use crate::{
    agent::AgentInstance,
    error::DaemonError,
    history::OperationalHistoryStore,
    provider::{ProviderPromptSignalBatch, RuntimeProviderRun},
    session::PromptQueueItem,
};

pub(crate) fn should_record(batch: &ProviderPromptSignalBatch) -> bool {
    batch.resolved_usage.is_some() || batch.prompt_completed || batch.terminal_failure.is_some()
}

pub(crate) fn provider_batch(
    store: &OperationalHistoryStore,
    run: &RuntimeProviderRun,
    agent: &AgentInstance,
    prompt: &PromptQueueItem,
    batch: &ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let session_id = run.session_id();
    let provider_run_id = run.id();
    let agent_id = agent.id();
    let previous = report::latest(store, session_id, prompt.id(), provider_run_id)?;
    let counters = batch.resolved_usage;
    let mut turn = TurnUsage {
        session_id: session_id.into(),
        agent_id: agent_id.into(),
        parent_agent_id: agent.controlled_by_metaagent_id().map(str::to_string),
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
    let metadata =
        std::collections::BTreeMap::from([(report::METADATA_KEY.into(), serde_json::json!(turn))]);
    store.append_operational_event(
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
