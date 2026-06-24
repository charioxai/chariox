use super::{ObservedExternalProviderTurn, ObservedExternalProviderTurnRole};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalProviderObservationPolicy<'a> {
    provider: &'a str,
}

impl<'a> ExternalProviderObservationPolicy<'a> {
    pub(crate) fn for_provider(provider: &'a str) -> Self {
        Self { provider }
    }

    pub(crate) fn uses_explicit_completion(self) -> bool {
        matches!(self.provider, "codex" | "opencode")
    }

    pub(crate) fn status_settles(self, text: &str) -> bool {
        text.starts_with("codex task_complete")
            || text.starts_with("codex event turn_aborted")
            || text.contains("\"type\":\"turn_aborted\"")
            || text.contains("\"type\": \"turn_aborted\"")
            || text.starts_with("claude message completed")
            || text.starts_with("opencode message completed")
    }

    pub(crate) fn status_is_passive_telemetry(self, text: &str) -> bool {
        self.provider == "claude"
            && (text.starts_with("claude last-prompt") || text.starts_with("claude ai-title"))
    }

    pub(crate) fn turn_is_passive_telemetry(self, turn: &ObservedExternalProviderTurn) -> bool {
        turn.role == ObservedExternalProviderTurnRole::Status
            && self.status_is_passive_telemetry(&turn.text)
    }

    pub(crate) fn latest_effective_turn_settles(
        self,
        turns: &[ObservedExternalProviderTurn],
    ) -> bool {
        let Some(latest) = turns
            .iter()
            .rev()
            .find(|turn| !self.turn_is_passive_telemetry(turn))
            .or_else(|| turns.last())
        else {
            return false;
        };
        match latest.role {
            ObservedExternalProviderTurnRole::Status => self.status_settles(&latest.text),
            ObservedExternalProviderTurnRole::Assistant
            | ObservedExternalProviderTurnRole::User
            | ObservedExternalProviderTurnRole::Reasoning
            | ObservedExternalProviderTurnRole::Tool => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_and_opencode_require_explicit_completion() {
        assert!(ExternalProviderObservationPolicy::for_provider("codex").uses_explicit_completion());
        assert!(
            ExternalProviderObservationPolicy::for_provider("opencode").uses_explicit_completion()
        );
        assert!(
            !ExternalProviderObservationPolicy::for_provider("claude").uses_explicit_completion()
        );
    }

    #[test]
    fn completion_and_abort_statuses_settle_turns() {
        for (provider, text) in [
            ("codex", "codex task_complete\n{}"),
            (
                "codex",
                "codex event turn_aborted {\"type\":\"turn_aborted\"}",
            ),
            ("claude", "claude message completed\n{}"),
            ("opencode", "opencode message completed\n{}"),
        ] {
            let policy = ExternalProviderObservationPolicy::for_provider(provider);
            assert!(
                policy.latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                }]),
                "{provider} status should settle"
            );
        }
    }

    #[test]
    fn claude_passive_telemetry_does_not_hide_prior_completion() {
        let policy = ExternalProviderObservationPolicy::for_provider("claude");
        assert!(
            policy.turn_is_passive_telemetry(&ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            })
        );
        assert!(policy.latest_effective_turn_settles(&[
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "claude message completed\n{}".to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            },
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "claude ai-title {\"title\":\"Title\"}".to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            },
        ]));
    }
}
