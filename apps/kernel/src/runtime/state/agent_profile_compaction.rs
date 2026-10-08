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
pub(super) const COMPACT_TIMEOUT: Duration = Duration::from_secs(300);

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
        // Both local and leased callers hold the idle profile-transition claim.
        if !needs_compaction(&run, target_provider, target_account, target_model) {
            return;
        }
        self.owned.record_notice(
            session_id,
            Some(run.id()),
            self.owned
                .attachment_store
                .list_session_attachment_ids(session_id),
            "Compacting the Claude session before changing to a smaller context window…",
        );
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
/// Its context figure is the last API call input, including cached input.
fn needs_compaction(
    run: &RuntimeProviderRun,
    target_provider: &str,
    target_account: &str,
    target_model: &str,
) -> bool {
    let claude =
        |provider: &str| crate::provider::canonical_provider_family(provider) == Some("claude");
    let window = effective_window(run, target_provider, target_model);
    matches!(
        run.state(),
        ProviderRunState::Running | ProviderRunState::Parked
    ) && claude(run.provider())
        && claude(target_provider)
        && run.account_profile() == target_account
        && crate::provider::provider_run_uses_structured_prompt_io(run)
        && window < effective_window(run, run.provider(), run.model())
        && run
            .usage()
            .context_tokens
            .is_some_and(|tokens| tokens * 100 > window * COMPACT_ABOVE_WINDOW_PERCENT)
}

// Source and target share this account's launch environment. Honor Claude's
// documented cap instead of compacting between two effectively equal windows.
fn effective_window(run: &RuntimeProviderRun, provider: &str, model: &str) -> u64 {
    crate::provider::effective_model_context_window_tokens(
        provider,
        model,
        crate::provider::claude_1m_context_disabled(run),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_run(model: &str, context_tokens: Option<u64>) -> RuntimeProviderRun {
        claude_run_for_provider("claude", model, context_tokens)
    }

    fn claude_run_for_provider(
        provider: &str,
        model: &str,
        context_tokens: Option<u64>,
    ) -> RuntimeProviderRun {
        let request = crate::provider::LaunchProviderRequest::new(
            "session", "claude", provider, "work", model,
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
    fn claude_headless_is_not_compacted_through_structured_io() {
        let run = claude_run_for_provider("claude-headless", "sonnet[1m]", Some(320_000));
        assert!(!needs_compaction(&run, "claude-headless", "work", "haiku"));
    }

    #[test]
    fn only_a_session_too_large_for_the_smaller_claude_window_is_compacted_first() {
        // Claude resolves `sonnet[1m]` to its model id; the run keeps the `[1m]`.
        let large = claude_run("claude-sonnet-5-5[1m]", Some(320_000));

        assert!(needs_compaction(&large, "claude", "work", "haiku"));
        assert!(!needs_compaction(&large, "claude", "work", "sonnet"));
        assert!(!needs_compaction(
            &claude_run("claude-sonnet-5-5[1m]", Some(90_000)),
            "claude",
            "work",
            "haiku"
        ));
        assert!(!needs_compaction(
            &claude_run("sonnet[1m]", None),
            "claude",
            "work",
            "haiku"
        ));
        assert!(!needs_compaction(&large, "claude", "work", "opus[1m]"));
        assert!(!needs_compaction(&large, "claude", "other", "haiku"));
        assert!(!needs_compaction(&large, "codex", "work", "gpt-6"));
        assert!(!needs_compaction(
            &claude_run("sonnet", Some(190_000)),
            "claude",
            "work",
            "opus"
        ));
    }
}
