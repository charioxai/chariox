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
use crate::runtime::agent_utility_executor::{
    run_provider_utility_prompt, AgentUtilityPromptParts,
};

use super::super::project_environment_export::MetadataUtilityScratch;
use super::super::{KernelRuntimeOwnedState, KernelRuntimeState};
use super::brief::{brief_prompt, parse_brief, transcript_chunks};
use super::PendingAgentContextHandoff;

/// How long a dispatch waits for its brief before it falls back.
const BRIEF_DEADLINE: Duration = Duration::from_secs(120);
/// Writes a Codex brief in about 5 s where the default gpt-5.5 took up to 19.
const CODEX_BRIEF_MODEL: &str = "gpt-6-luna";

impl KernelRuntimeState {
    /// `handoff` with its brief brought up to date through the history before
    /// the dispatching prompt, when it is a provider-switch handoff whose
    /// target harness can write one.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime::state) async fn with_current_handoff_brief(
        &self,
        owned: &KernelRuntimeOwnedState,
        mut handoff: PendingAgentContextHandoff,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        target_run: &RuntimeProviderRun,
        prompt_is_current: impl Fn() -> bool + Send + Sync,
    ) -> PendingAgentContextHandoff {
        if !handoff.derived || !writes_handoff_briefs(target_run) {
            return handoff;
        }
        let started = Instant::now();
        let update = self
            .update_handoff_brief(
                owned,
                &handoff,
                session_id,
                agent_id,
                prompt_id,
                target_run,
                started,
                &prompt_is_current,
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
                handoff.conversation.brief = brief;
            }
            Err(error) => {
                // Every completed fold is durable; use it even if a later fold failed.
                handoff.conversation.brief = owned.stored_handoff_brief(session_id, agent_id);
                crate::logging::warn_with_fields(
                "daemon.provider_context_handoff",
                "failed to update the agent's handoff brief; the handoff keeps the stored brief",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "target_provider_run_id": target_run.id(),
                    "elapsed_ms": elapsed_ms,
                    "error": error.to_string(),
                }),
                );
            }
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
        prompt_is_current: &(impl Fn() -> bool + Sync),
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
        let config = owned.config_projection.snapshot();
        let configured = crate::provider::canonical_provider_family(target_run.provider())
            .and_then(|family| config.user_config.history.handoff.brief_model(family));
        utility_run.set_model(brief_model(configured, handoff, target_run));
        if owned
            .pending_agent_context_handoffs
            .brief_model_is_unavailable(&utility_run)
        {
            utility_run.set_model(target_run.model().to_string());
        }
        utility_run.set_variant(Some("low".to_string()));
        utility_run.set_metadata_only_discovery(scratch.0.clone());
        let mut calls = 0;
        for (part, chunk) in chunks.iter().enumerate() {
            if !prompt_is_current() {
                return Ok((brief, calls));
            }
            if !chunk.text.is_empty() {
                let (output, part_calls) = call_with_model_fallback(
                    &owned.pending_agent_context_handoffs,
                    &mut utility_run,
                    target_run,
                    brief_prompt(brief.as_deref(), chunk, part, chunks.len()),
                    calls == 0,
                    |run, prompt| async move {
                        if !prompt_is_current() {
                            return Err(brief_error("the dispatching prompt is no longer active"));
                        }
                        self.brief_call(&run, prompt, started).await
                    },
                )
                .await?;
                calls += part_calls;
                // An in-flight utility may finish after cancellation. Its fold
                // must not race a later prompt's shared brief and watermark.
                if !prompt_is_current() {
                    return Ok((brief, calls));
                }
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

impl KernelRuntimeState {
    /// One utility call within what is left of the brief deadline.
    async fn brief_call(
        &self,
        utility_run: &RuntimeProviderRun,
        prompt: AgentUtilityPromptParts,
        started: Instant,
    ) -> Result<String, DaemonError> {
        let remaining = BRIEF_DEADLINE
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| brief_error("the brief deadline passed"))?;
        run_provider_utility_prompt(
            self,
            utility_run.clone(),
            prompt,
            "update handoff brief",
            ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
            remaining,
        )
        .await
    }
}

/// Codex app-server runs and Chariox Claude runs can run a metadata-only
/// utility turn; other harnesses fall back to the deterministic packet.
pub(in crate::runtime::state) fn writes_handoff_briefs(run: &RuntimeProviderRun) -> bool {
    run.adapter_key() == "codex"
        || crate::provider::provider_run_uses_runtime_structured_utility_prompt(run)
}

/// The configured brief model for the target harness, else on Codex its fast
/// default, else the source model when the harness can run it, else the
/// target model.
pub(super) fn brief_model(
    configured: Option<&str>,
    handoff: &PendingAgentContextHandoff,
    target_run: &RuntimeProviderRun,
) -> String {
    let family = crate::provider::canonical_provider_family(target_run.provider());
    configured
        .map(str::to_string)
        .or_else(|| (family == Some("codex")).then(|| CODEX_BRIEF_MODEL.to_string()))
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

/// One fold, with a single retry only for a definite model rejection.
async fn call_with_model_fallback<F, Fut>(
    store: &super::PendingAgentContextHandoffStore,
    utility_run: &mut RuntimeProviderRun,
    target_run: &RuntimeProviderRun,
    prompt: AgentUtilityPromptParts,
    allow_fallback: bool,
    mut call: F,
) -> Result<(String, usize), DaemonError>
where
    F: FnMut(RuntimeProviderRun, AgentUtilityPromptParts) -> Fut,
    Fut: std::future::Future<Output = Result<String, DaemonError>>,
{
    match call(utility_run.clone(), prompt.clone()).await {
        Err(error)
            if allow_fallback
                && fallback_after_unavailable_model(store, utility_run, target_run, &error) =>
        {
            crate::logging::warn_with_fields(
                "daemon.provider_context_handoff",
                "the brief model is unavailable; briefing with the target model",
                serde_json::json!({"brief_model": utility_run.model(), "target_provider_run_id": target_run.id(), "error": error.to_string()}),
            );
            utility_run.set_model(target_run.model().to_string());
            call(utility_run.clone(), prompt)
                .await
                .map(|output| (output, 2))
        }
        result => result.map(|output| (output, 1)),
    }
}

/// Only a definite unsupported-model rejection justifies changing models.
/// Network, quota, timeout and other transient failures retain the configured model.
fn fallback_after_unavailable_model(
    store: &super::PendingAgentContextHandoffStore,
    utility_run: &RuntimeProviderRun,
    target_run: &RuntimeProviderRun,
    error: &DaemonError,
) -> bool {
    if utility_run.model() == target_run.model() {
        return false;
    }
    let message = error.to_string().to_ascii_lowercase();
    let unsupported = [
        "unsupported model",
        "model_not_found",
        "model not found",
        "model does not exist",
        "model is not supported",
        "model is not available for this account",
        "not supported when using codex with a chatgpt account",
    ]
    .iter()
    .any(|reason| message.contains(reason));
    if unsupported {
        store.remember_unavailable_brief_model(utility_run);
    }
    unsupported
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(account: &str, model: &str) -> RuntimeProviderRun {
        RuntimeProviderRun::new(
            "run",
            &crate::provider::LaunchProviderRequest::new("s", "codex", "codex", account, model),
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "codex".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }

    #[tokio::test]
    async fn the_first_fold_retries_an_unavailable_model_once_but_not_a_transient_error() {
        let store = super::super::PendingAgentContextHandoffStore::default();
        let target = run("work", "gpt-5.5");
        let mut utility = run("work", "gpt-6-luna");
        let mut models = Vec::new();
        let prompt = || AgentUtilityPromptParts {
            visible_user_prompt: "fold".into(),
            hidden_system_context: "brief".into(),
        };
        let (output, calls) = call_with_model_fallback(
            &store,
            &mut utility,
            &target,
            prompt(),
            true,
            |run, input| {
                models.push(run.model().to_string());
                assert_eq!(input.visible_user_prompt, "fold");
                std::future::ready(if run.model() == "gpt-6-luna" {
                    Err(brief_error("unsupported model gpt-6-luna"))
                } else {
                    Ok("folded".into())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(output, "folded");
        assert_eq!(calls, 2);
        assert_eq!(models, ["gpt-6-luna", "gpt-5.5"]);
        assert!(store.brief_model_is_unavailable(&run("work", "gpt-6-luna")));
        let mut calls = 0;
        let result = call_with_model_fallback(
            &store,
            &mut run("personal", "gpt-6-luna"),
            &target,
            prompt(),
            true,
            |_, _| {
                calls += 1;
                std::future::ready(Err(brief_error("rate limit exceeded")))
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn unavailable_model_fallback_is_account_scoped_and_transient_errors_do_not_switch() {
        let store = super::super::PendingAgentContextHandoffStore::default();
        let utility = run("work", "gpt-6-luna");
        let target = run("work", "gpt-5.5");
        for message in [
            "network timeout",
            "rate limit exceeded",
            "quota exhausted",
            "server overloaded",
            "model is not available at this time",
        ] {
            assert!(!fallback_after_unavailable_model(
                &store,
                &utility,
                &target,
                &brief_error(message)
            ));
            assert!(!store.brief_model_is_unavailable(&utility));
        }
        assert!(fallback_after_unavailable_model(
            &store,
            &utility,
            &target,
            &brief_error("unsupported model gpt-6-luna")
        ));
        assert!(store.brief_model_is_unavailable(&utility));
        assert!(!store.brief_model_is_unavailable(&run("personal", "gpt-6-luna")));
        assert!(!store.brief_model_is_unavailable(&run("work", "different-model")));
        assert!(!fallback_after_unavailable_model(
            &store,
            &target,
            &target,
            &brief_error("unsupported model")
        ));
    }
}
