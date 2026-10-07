//! Before a model change within Claude moves a session to a smaller context
//! window, the source model compacts the session through Claude's own
//! `/compact`. Claude compacts only near the window of the model it runs, so
//! a transcript larger than the new window would otherwise be summarized by
//! the smaller model, which drops the oldest part first. Codex compacts with
//! the previous model on its own.

use std::time::{Duration, Instant};

use crate::provider::{ProviderRunState, ProviderUtilityExecutionPolicy, RuntimeProviderRun};

use super::KernelRuntimeState;

/// The share of the new window above which the session is compacted first.
const COMPACT_ABOVE_WINDOW_PERCENT: u64 = 75;
const COMPACT_TIMEOUT: Duration = Duration::from_secs(300);

impl KernelRuntimeState {
    /// Compacts the agent's live Claude session with its current model when
    /// the new profile keeps the session on a window it would not fit. A
    /// failure leaves the session to Claude's own compaction.
    pub(super) async fn compact_before_window_downshift(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        provider: Option<&str>,
        account_profile: Option<&str>,
        model: Option<&str>,
    ) {
        let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent.id())
        else {
            return;
        };
        let target_provider = provider.unwrap_or(agent.provider());
        let target_account = account_profile.unwrap_or(agent.provider_account_profile());
        let Some(target_model) = model.or(agent.model()) else {
            return;
        };
        let idle = self
            .owned
            .session_store
            .get_session(session_id)
            .is_ok_and(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent.id())
                    .is_none()
            });
        if !idle || !needs_compaction(&run, target_provider, target_account, target_model) {
            return;
        }
        let started = Instant::now();
        let result = self
            .run_structured_provider_utility_prompt(
                run.clone(),
                "/compact".to_string(),
                String::new(),
                COMPACT_TIMEOUT,
                ProviderUtilityExecutionPolicy::SessionCommand,
            )
            .await;
        let fields = serde_json::json!({
            "session_id": session_id,
            "agent_id": agent.id(),
            "provider_run_id": run.id(),
            "source_model": run.model(),
            "target_model": target_model,
            "context_tokens": run.usage().context_tokens,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        });
        match result {
            Ok(_) => crate::logging::info_with_fields(
                "daemon.provider_context_handoff",
                "compacted the session before a smaller context window",
                fields,
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.provider_context_handoff",
                "failed to compact the session before a smaller context window",
                serde_json::json!({ "error": error.to_string(), "details": fields }),
            ),
        }
    }
}

/// A live Chariox Claude run whose session the profile change keeps, on a
/// smaller window that its last turn's context nearly fills or exceeds.
fn needs_compaction(
    run: &RuntimeProviderRun,
    target_provider: &str,
    target_account: &str,
    target_model: &str,
) -> bool {
    let claude =
        |provider: &str| crate::provider::canonical_provider_family(provider) == Some("claude");
    let window = crate::provider::model_context_window_tokens(target_provider, target_model);
    matches!(
        run.state(),
        ProviderRunState::Running | ProviderRunState::Parked
    ) && claude(run.provider())
        && claude(target_provider)
        && run.account_profile() == target_account
        && crate::provider::provider_run_uses_runtime_structured_utility_prompt(run)
        && window < crate::provider::model_context_window_tokens(run.provider(), run.model())
        && run
            .usage()
            .context_tokens
            .is_some_and(|tokens| tokens * 100 > window * COMPACT_ABOVE_WINDOW_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_run(model: &str, context_tokens: Option<u64>) -> RuntimeProviderRun {
        let request = crate::provider::LaunchProviderRequest::new(
            "session", "claude", "claude", "work", model,
        )
        .with_agent_id("agent");
        let mut run = RuntimeProviderRun::new(
            "run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "claude".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        run.set_usage(crate::provider::ProviderRunTokenUsage {
            context_tokens,
            ..Default::default()
        });
        run
    }

    #[test]
    fn only_a_session_too_large_for_the_smaller_claude_window_is_compacted_first() {
        let large = claude_run("sonnet[1m]", Some(320_000));

        assert!(needs_compaction(&large, "claude", "work", "sonnet"));
        assert!(!needs_compaction(
            &claude_run("sonnet[1m]", Some(90_000)),
            "claude",
            "work",
            "sonnet"
        ));
        assert!(!needs_compaction(
            &claude_run("sonnet[1m]", None),
            "claude",
            "work",
            "sonnet"
        ));
        assert!(!needs_compaction(&large, "claude", "work", "opus[1m]"));
        assert!(!needs_compaction(&large, "claude", "other", "sonnet"));
        assert!(!needs_compaction(&large, "codex", "work", "gpt-6"));
        assert!(!needs_compaction(
            &claude_run("sonnet", Some(190_000)),
            "claude",
            "work",
            "opus"
        ));
    }
}
