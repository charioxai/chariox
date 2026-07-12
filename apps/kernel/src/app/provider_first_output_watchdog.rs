use std::collections::BTreeMap;
use std::time::Duration;

use crate::app::{ActivePromptState, ActiveTurnPhase, ActiveTurnState};

const PROVIDER_FIRST_OUTPUT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub(crate) const PROVIDER_OUTPUT_TIMEOUT_MS: u64 = PROVIDER_FIRST_OUTPUT_TIMEOUT.as_millis() as u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderFirstOutputTimeoutCandidate {
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderInactivityTimeoutCandidate {
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) elapsed_ms: u64,
}

pub(crate) fn provider_first_output_timeout_candidates(
    session_id: &str,
    active_turns: impl IntoIterator<Item = ActiveTurnState>,
    prompt_activity: &BTreeMap<String, ActivePromptState>,
    mut provider_run_is_waiting: impl FnMut(&ActiveTurnState) -> bool,
    mut active_prompt_matches: impl FnMut(&ActiveTurnState) -> bool,
) -> Vec<ProviderFirstOutputTimeoutCandidate> {
    let now_ms = crate::session::unix_epoch_ms();
    let timeout_ms = PROVIDER_OUTPUT_TIMEOUT_MS;
    active_turns
        .into_iter()
        .filter_map(|turn| {
            if turn.session_id != session_id {
                return None;
            }
            if !matches!(
                turn.phase,
                ActiveTurnPhase::Accepted | ActiveTurnPhase::AwaitingFirstOutput
            ) {
                return None;
            }
            if !prompt_activity
                .get(&turn.provider_run_id)
                .is_some_and(|activity| !activity.saw_response_content)
            {
                return None;
            }
            if !provider_run_is_waiting(&turn) || !active_prompt_matches(&turn) {
                return None;
            }
            let elapsed_ms = now_ms.saturating_sub(turn.started_at_ms);
            (elapsed_ms >= timeout_ms).then_some(ProviderFirstOutputTimeoutCandidate {
                provider_run_id: turn.provider_run_id,
                agent_id: turn.agent_id,
                elapsed_ms,
            })
        })
        .collect()
}

pub(crate) fn provider_inactivity_timeout_candidates(
    session_id: &str,
    active_turns: impl IntoIterator<Item = ActiveTurnState>,
    prompt_activity: &BTreeMap<String, ActivePromptState>,
    mut provider_run_is_waiting: impl FnMut(&ActiveTurnState) -> bool,
    mut active_prompt_matches: impl FnMut(&ActiveTurnState) -> bool,
) -> Vec<ProviderInactivityTimeoutCandidate> {
    let timeout_ms = PROVIDER_OUTPUT_TIMEOUT_MS;
    active_turns
        .into_iter()
        .filter_map(|turn| {
            if turn.session_id != session_id {
                return None;
            }
            if !matches!(
                turn.phase,
                ActiveTurnPhase::Streaming | ActiveTurnPhase::Settling
            ) {
                return None;
            }
            let elapsed_ms = prompt_activity
                .get(&turn.provider_run_id)
                .and_then(|activity| activity.last_output_at)
                .map(|last_output_at| last_output_at.elapsed().as_millis() as u64)?;
            if elapsed_ms < timeout_ms {
                return None;
            }
            if !provider_run_is_waiting(&turn) || !active_prompt_matches(&turn) {
                return None;
            }
            Some(ProviderInactivityTimeoutCandidate {
                provider_run_id: turn.provider_run_id,
                agent_id: turn.agent_id,
                elapsed_ms,
            })
        })
        .collect()
}

pub(crate) fn provider_first_output_timeout_diagnostic(elapsed_ms: u64) -> String {
    format!(
        "Provider prompt produced no output for {} seconds after launch; the provider may be stuck. Arroba closed this turn so the agent can be retried.",
        elapsed_ms / 1000
    )
}

pub(crate) fn provider_inactivity_timeout_diagnostic(elapsed_ms: u64) -> String {
    format!(
        "Provider prompt produced no output for {} seconds after its last activity; the provider may be stuck. Arroba closed this turn so the agent can be retried.",
        elapsed_ms / 1000
    )
}
