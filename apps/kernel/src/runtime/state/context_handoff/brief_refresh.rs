//! Brings an agent's handoff brief up to date before a dispatch carries the
//! derived handoff. The update runs through the target run's official harness
//! in a fresh, tool-free session that sees only the supplied text, and folds
//! the history after the brief's watermark into it, oldest first. Each folded
//! chunk is stored at once, so a later switch resumes from there. A failure or
//! the deadline leaves the stored brief and the deterministic packet.

use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::history::{AgentHandoffBrief, HistoryEventKind};
use crate::provider::{ProviderUtilityExecutionPolicy, RuntimeProviderRun};
use crate::runtime::agent_utility_executor::run_provider_utility_prompt;

use super::super::project_environment_export::MetadataUtilityScratch;
use super::super::{KernelRuntimeOwnedState, KernelRuntimeState};
use super::brief::{brief_prompt, parse_brief, transcript_chunks};
use super::PendingAgentContextHandoff;

/// How long a dispatch waits for its brief before it falls back.
const BRIEF_DEADLINE: Duration = Duration::from_secs(120);

impl KernelRuntimeState {
    /// `handoff` with its brief brought up to date through the history before
    /// the dispatching prompt, when it is a provider-switch handoff whose
    /// target harness can write one.
    pub(in crate::runtime::state) async fn with_current_handoff_brief(
        &self,
        owned: &KernelRuntimeOwnedState,
        mut handoff: PendingAgentContextHandoff,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        target_run: &RuntimeProviderRun,
    ) -> PendingAgentContextHandoff {
        if !handoff.derived || !writes_handoff_briefs(target_run) {
            return handoff;
        }
        let started = Instant::now();
        let update = self
            .update_handoff_brief(
                owned, &handoff, session_id, agent_id, prompt_id, target_run, started,
            )
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match update {
            Ok((brief, calls)) => {
                crate::logging::info_with_fields(
                    "daemon.provider_context_handoff",
                    "updated the agent's handoff brief",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "target_provider_run_id": target_run.id(),
                        "elapsed_ms": elapsed_ms,
                        "utility_calls": calls,
                        "brief_bytes": brief.as_ref().map(String::len),
                    }),
                );
                if brief.is_some() {
                    handoff.conversation.brief = brief;
                }
            }
            Err(error) => crate::logging::warn_with_fields(
                "daemon.provider_context_handoff",
                "failed to update the agent's handoff brief; the handoff keeps the stored brief",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "target_provider_run_id": target_run.id(),
                    "elapsed_ms": elapsed_ms,
                    "error": error.to_string(),
                }),
            ),
        }
        handoff
    }

    /// The brief through the history before `prompt_id`, and the utility
    /// calls it took.
    #[allow(clippy::too_many_arguments)]
    async fn update_handoff_brief(
        &self,
        owned: &KernelRuntimeOwnedState,
        handoff: &PendingAgentContextHandoff,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        target_run: &RuntimeProviderRun,
        started: Instant,
    ) -> Result<(Option<String>, usize), DaemonError> {
        let history = &owned.operational_history_store;
        let stored = history.load_agent_handoff_brief(session_id, agent_id)?;
        let after = stored
            .as_ref()
            .map_or(0, |stored| stored.covered_through_sequence);
        let events = history.load_session_events_for_agent_sequence_range(
            session_id,
            agent_id,
            after + 1,
            i64::MAX as u64,
        )?;
        let mut events = owned
            .room_secret_observations
            .protect_history_events(events);
        // The dispatching prompt is the request itself, not history to brief.
        if let Some(end) = events
            .iter()
            .find(|event| {
                event.kind == HistoryEventKind::UserPrompt
                    && event.prompt_id.as_deref() == Some(prompt_id)
            })
            .map(|event| event.sequence)
        {
            events.retain(|event| event.sequence < end);
        }
        let mut brief = stored.map(|stored| stored.brief);
        let chunks = transcript_chunks(&events);
        if chunks.iter().all(|chunk| chunk.text.is_empty()) {
            return Ok((brief, 0));
        }
        let scratch = MetadataUtilityScratch::new(
            &owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )?;
        let mut utility_run = target_run.clone();
        utility_run.set_model(brief_model(owned, handoff, target_run));
        utility_run.set_variant(Some("low".to_string()));
        utility_run.set_metadata_only_discovery(scratch.0.clone());
        let mut calls = 0;
        for (part, chunk) in chunks.iter().enumerate() {
            if !chunk.text.is_empty() {
                let remaining = BRIEF_DEADLINE
                    .checked_sub(started.elapsed())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or_else(|| brief_error("the brief deadline passed"))?;
                calls += 1;
                let output = tokio::time::timeout(
                    remaining,
                    run_provider_utility_prompt(
                        self,
                        utility_run.clone(),
                        brief_prompt(brief.as_deref(), chunk, part, chunks.len()),
                        "update handoff brief",
                        ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
                    ),
                )
                .await
                .map_err(|_| brief_error("the brief deadline passed"))??;
                brief = Some(
                    parse_brief(&output)
                        .ok_or_else(|| brief_error("the utility answer is not a handoff brief"))?,
                );
            }
            if let Some(brief) = &brief {
                history.save_agent_handoff_brief(
                    session_id,
                    agent_id,
                    &AgentHandoffBrief {
                        brief: brief.clone(),
                        covered_through_sequence: chunk.through_sequence,
                    },
                )?;
            }
        }
        Ok((brief, calls))
    }
}

/// Codex app-server runs and Chariox Claude runs can run a metadata-only
/// utility turn; other harnesses fall back to the deterministic packet.
fn writes_handoff_briefs(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "codex"
        || crate::provider::provider_run_uses_runtime_structured_utility_prompt(run)
}

/// The configured brief model for the target harness, else the source model
/// when the harness can run it, else the target model.
fn brief_model(
    owned: &KernelRuntimeOwnedState,
    handoff: &PendingAgentContextHandoff,
    target_run: &RuntimeProviderRun,
) -> String {
    let family = crate::provider::canonical_provider_family(target_run.provider());
    let configured = family.and_then(|family| {
        owned
            .config_projection
            .snapshot()
            .user_config
            .history
            .handoff
            .brief_model(family)
            .map(str::to_string)
    });
    configured
        .or_else(|| {
            (family.is_some()
                && crate::provider::canonical_provider_family(&handoff.source_provider) == family
                && !handoff.source_model.trim().is_empty())
            .then(|| handoff.source_model.clone())
        })
        .unwrap_or_else(|| target_run.model().to_string())
}

fn brief_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "update handoff brief",
        message: message.to_string(),
    }
}
