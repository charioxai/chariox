//! MP-08 / MP-10 / MP-11: durable, deduplicated prompt accounting.
use super::{quote, Usage, PRICE_TABLE_DATE, PRICE_TABLE_VERSION};
use crate::error::DaemonError;
use crate::history::{HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const METADATA_KEY: &str = "provider_usage_accounting_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub session_id: String,
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub prompt_id: String,
    pub provider_run_id: String,
    pub provider: String,
    pub model: String,
    pub completed: bool,
    pub usage: Option<Usage>,
    pub provider_counters: Option<Usage>,
    /// Decimal string preserves integer nanodollars through JavaScript clients.
    pub api_equivalent_nanodollars: Option<String>,
    pub price_table_version: u32,
    pub price_table_date: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub turns: usize,
    pub unavailable_turns: usize,
    pub usage: Usage,
    pub api_equivalent_nanodollars: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUsageReport {
    pub session_id: String,
    pub turns: Vec<TurnUsage>,
    pub total: UsageTotals,
    pub agents: BTreeMap<String, UsageTotals>,
    pub delegation_trees: BTreeMap<String, UsageTotals>,
}

pub fn totals<'a>(turns: impl Iterator<Item = &'a TurnUsage>) -> UsageTotals {
    let mut result = UsageTotals {
        turns: 0,
        unavailable_turns: 0,
        usage: Usage::zero(),
        api_equivalent_nanodollars: Some("0".into()),
    };
    fn add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
        a?.checked_add(b?)
    }
    for turn in turns {
        result.turns += 1;
        let u = turn.usage.unwrap_or_default();
        if u.input.is_none() || u.output.is_none() {
            result.unavailable_turns += 1;
        }
        result.usage = Usage {
            input: add(result.usage.input, u.input),
            cached_input: add(result.usage.cached_input, u.cached_input),
            output: add(result.usage.output, u.output),
            reasoning: add(result.usage.reasoning, u.reasoning),
            cache_write: add(result.usage.cache_write, u.cache_write),
            cache_write_5m: add(result.usage.cache_write_5m, u.cache_write_5m),
            cache_write_1h: add(result.usage.cache_write_1h, u.cache_write_1h),
        };
        result.api_equivalent_nanodollars = result
            .api_equivalent_nanodollars
            .as_ref()
            .and_then(|s| s.parse::<u128>().ok())
            .zip(
                turn.api_equivalent_nanodollars
                    .as_ref()
                    .and_then(|s| s.parse::<u128>().ok()),
            )
            .and_then(|(a, b)| a.checked_add(b))
            .map(|n| n.to_string());
    }
    result
}

pub fn load(
    store: &OperationalHistoryStore,
    session_id: &str,
) -> Result<SessionUsageReport, DaemonError> {
    let mut latest = BTreeMap::new();
    let mut after = None;
    loop {
        let events = store.query_events(HistoryEventQuery {
            session_id: Some(session_id.into()),
            after_sequence: after,
            limit: Some(500),
            ..Default::default()
        })?;
        let count = events.len();
        for event in events {
            after = Some(event.sequence);
            // An older or failed provider prompt without counters is unavailable,
            // never an implicit zero. Only bound provider prompts are admitted.
            if event.kind == HistoryEventKind::UserPrompt {
                if let (
                    Some(prompt_id),
                    Some(provider_run_id),
                    Some(agent_id),
                    Some(provider),
                    Some(model),
                ) = (
                    &event.prompt_id,
                    &event.provider_run_id,
                    &event.agent_id,
                    &event.provider,
                    &event.model,
                ) {
                    latest
                        .entry((prompt_id.clone(), provider_run_id.clone()))
                        .or_insert_with(|| TurnUsage {
                            session_id: session_id.into(),
                            agent_id: agent_id.clone(),
                            parent_agent_id: None,
                            prompt_id: prompt_id.clone(),
                            provider_run_id: provider_run_id.clone(),
                            provider: provider.clone(),
                            model: model.clone(),
                            completed: false,
                            usage: None,
                            provider_counters: None,
                            api_equivalent_nanodollars: None,
                            price_table_version: PRICE_TABLE_VERSION,
                            price_table_date: PRICE_TABLE_DATE.into(),
                        });
                }
            }
            if let Some(value) = event.metadata.get(METADATA_KEY) {
                let turn: TurnUsage = serde_json::from_value(value.clone()).map_err(|error| {
                    DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.into()),
                        operation: "decode provider usage",
                        message: error.to_string(),
                    }
                })?;
                latest.insert((turn.prompt_id.clone(), turn.provider_run_id.clone()), turn);
            }
        }
        if count < 500 {
            break;
        }
    }
    Ok(from_turns(session_id, latest.into_values().collect()))
}

pub fn from_turns(session_id: &str, turns: Vec<TurnUsage>) -> SessionUsageReport {
    let mut agents = BTreeMap::new();
    let parents: BTreeMap<_, _> = turns
        .iter()
        .map(|t| (t.agent_id.clone(), t.parent_agent_id.clone()))
        .collect();
    let mut delegation_trees = BTreeMap::new();
    for agent in parents.keys() {
        agents.insert(
            agent.clone(),
            totals(turns.iter().filter(|t| &t.agent_id == agent)),
        );
        delegation_trees.insert(
            agent.clone(),
            totals(turns.iter().filter(|t| {
                let mut cursor = Some(t.agent_id.as_str());
                let mut seen = std::collections::BTreeSet::new();
                while let Some(id) = cursor {
                    if !seen.insert(id) {
                        break;
                    }
                    if id == agent {
                        return true;
                    }
                    cursor = parents.get(id).and_then(|p| p.as_deref());
                }
                false
            })),
        );
    }
    let total = totals(turns.iter());
    SessionUsageReport {
        session_id: session_id.into(),
        turns,
        total,
        agents,
        delegation_trees,
    }
}

pub fn price(model: &str, usage: Option<Usage>) -> Option<String> {
    // No alias or context-band guess. Exact unbanded model quotes only.
    usage
        .and_then(|u| quote(model, None, &u))
        .map(|n| n.to_string())
}

pub fn stamp(turn: &mut TurnUsage) {
    turn.price_table_version = PRICE_TABLE_VERSION;
    turn.price_table_date = PRICE_TABLE_DATE.into();
    turn.api_equivalent_nanodollars = price(&turn.model, turn.usage);
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
