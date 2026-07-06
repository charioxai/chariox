use std::collections::BTreeSet;

use super::{ObservedExternalProviderTurn, ObservedExternalProviderTurnRole};
use crate::history::SessionHistoryExternalObservation;
use crate::provider::ProviderRunTokenUsage;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalProviderObservationPolicy<'a> {
    provider: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalProviderActivePromptSync<'turn> {
    pub(crate) active_prompt_turn: Option<&'turn ObservedExternalProviderTurn>,
    pub(crate) latest_observation_settles: bool,
    pub(crate) should_sync_active_prompt: bool,
}

#[derive(Debug, Clone, Copy)]
struct ExternalProviderObservationSpec {
    provider: &'static str,
    requires_explicit_completion: bool,
    settling_status_prefixes: &'static [&'static str],
    settling_status_fragments: &'static [&'static str],
    passive_status_prefixes: &'static [&'static str],
    projects_token_usage: bool,
}

const EXTERNAL_PROVIDER_OBSERVATION_SPECS: &[ExternalProviderObservationSpec] = &[
    ExternalProviderObservationSpec {
        provider: "codex",
        requires_explicit_completion: true,
        settling_status_prefixes: &["codex task_complete", "codex event turn_aborted"],
        settling_status_fragments: &["\"type\":\"turn_aborted\"", "\"type\": \"turn_aborted\""],
        passive_status_prefixes: &["codex token_count"],
        projects_token_usage: true,
    },
    ExternalProviderObservationSpec {
        provider: "claude",
        requires_explicit_completion: false,
        settling_status_prefixes: &["claude message completed"],
        settling_status_fragments: &[],
        passive_status_prefixes: &["claude last-prompt", "claude ai-title"],
        projects_token_usage: false,
    },
    ExternalProviderObservationSpec {
        provider: "opencode",
        requires_explicit_completion: true,
        settling_status_prefixes: &["opencode message completed"],
        settling_status_fragments: &[],
        passive_status_prefixes: &[],
        projects_token_usage: false,
    },
];

impl<'a> ExternalProviderObservationPolicy<'a> {
    pub(crate) fn for_provider(provider: &'a str) -> Self {
        Self { provider }
    }

    fn spec(self) -> Option<&'static ExternalProviderObservationSpec> {
        let provider = self.provider.trim();
        EXTERNAL_PROVIDER_OBSERVATION_SPECS
            .iter()
            .find(|spec| provider.eq_ignore_ascii_case(spec.provider))
    }

    pub(crate) fn uses_explicit_completion(self) -> bool {
        self.spec()
            .is_some_and(|spec| spec.requires_explicit_completion)
    }

    pub(crate) fn status_settles(self, text: &str) -> bool {
        self.spec().is_some_and(|spec| {
            spec.settling_status_prefixes
                .iter()
                .any(|prefix| text.starts_with(prefix))
                || spec
                    .settling_status_fragments
                    .iter()
                    .any(|fragment| text.contains(fragment))
        })
    }

    pub(crate) fn status_is_passive_telemetry(self, text: &str) -> bool {
        self.spec().is_some_and(|spec| {
            spec.passive_status_prefixes
                .iter()
                .any(|prefix| text.starts_with(prefix))
        })
    }

    pub(crate) fn status_usage(self, text: &str) -> Option<ProviderRunTokenUsage> {
        if !self.spec().is_some_and(|spec| spec.projects_token_usage) {
            return None;
        }
        let (header, payload) = text.split_once('\n')?;
        if header.trim() != "codex token_count" {
            return None;
        }
        let payload: serde_json::Value = serde_json::from_str(payload).ok()?;
        let context_tokens = first_u64_path(
            &payload,
            &[
                &["info", "total_token_usage", "total_tokens"],
                &["total_token_usage", "total_tokens"],
                &["info", "totalTokenUsage", "totalTokens"],
                &["totalTokenUsage", "totalTokens"],
                &["last", "total_tokens"],
                &["last", "totalTokens"],
            ],
        );
        let context_window = first_u64_path(
            &payload,
            &[
                &["info", "model_context_window"],
                &["info", "modelContextWindow"],
                &["model_context_window"],
                &["modelContextWindow"],
            ],
        );
        let context_tokens_with_window = match (context_tokens, context_window) {
            (Some(tokens), Some(window)) if tokens <= window => Some(tokens),
            _ => None,
        };
        (context_tokens.is_some() || context_window.is_some()).then_some(ProviderRunTokenUsage {
            total_tokens: context_tokens,
            last_tokens: context_tokens,
            context_tokens: context_tokens_with_window,
            context_window,
        })
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

    pub(crate) fn active_external_prompt_turn<'turn>(
        self,
        turns: &'turn [ObservedExternalProviderTurn],
        has_new_observations: bool,
        arroba_owned_provider_turn_ids: &BTreeSet<String>,
    ) -> Option<&'turn ObservedExternalProviderTurn> {
        if self.latest_effective_turn_settles(turns) {
            return None;
        }
        let latest = if has_new_observations {
            turns
                .iter()
                .rev()
                .find(|turn| turn.role == ObservedExternalProviderTurnRole::User)?
        } else {
            let latest = turns
                .iter()
                .rev()
                .find(|turn| !self.turn_is_passive_telemetry(turn))
                .or_else(|| turns.last())?;
            match latest.role {
                ObservedExternalProviderTurnRole::Assistant if !self.uses_explicit_completion() => {
                    return None;
                }
                ObservedExternalProviderTurnRole::Status if self.status_settles(&latest.text) => {
                    return None;
                }
                ObservedExternalProviderTurnRole::User => latest,
                ObservedExternalProviderTurnRole::Assistant
                | ObservedExternalProviderTurnRole::Reasoning
                | ObservedExternalProviderTurnRole::Tool
                | ObservedExternalProviderTurnRole::Status => turns
                    .iter()
                    .rev()
                    .find(|turn| turn.role == ObservedExternalProviderTurnRole::User)?,
            }
        };
        if arroba_owned_provider_turn_ids.contains(&latest.provider_turn_id_or_fallback()) {
            return None;
        }
        Some(latest)
    }

    pub(crate) fn active_prompt_sync<'turn>(
        self,
        turns: &'turn [ObservedExternalProviderTurn],
        changed_count: usize,
        active_relevant_changed_count: usize,
        allow_stable_settlement: bool,
        arroba_owned_provider_turn_ids: &BTreeSet<String>,
    ) -> ExternalProviderActivePromptSync<'turn> {
        let active_prompt_turn = self.active_external_prompt_turn(
            turns,
            active_relevant_changed_count > 0,
            arroba_owned_provider_turn_ids,
        );
        let latest_observation_settles = self.latest_effective_turn_settles(turns);
        let has_active_relevant_observation = turns
            .iter()
            .any(|turn| !self.turn_is_passive_telemetry(turn));
        let should_sync_active_prompt = active_prompt_turn.is_some()
            || latest_observation_settles
            || (allow_stable_settlement
                && has_active_relevant_observation
                && (changed_count == 0 || active_relevant_changed_count == 0));
        ExternalProviderActivePromptSync {
            active_prompt_turn,
            latest_observation_settles,
            should_sync_active_prompt,
        }
    }

    pub(crate) fn observation_for_turn(
        self,
        turn: &ObservedExternalProviderTurn,
    ) -> Option<SessionHistoryExternalObservation> {
        SessionHistoryExternalObservation {
            settles_active_prompt: turn.role == ObservedExternalProviderTurnRole::Status
                && self.status_settles(&turn.text),
            passive_telemetry: self.turn_is_passive_telemetry(turn),
        }
        .useful()
    }
}

fn first_u64_path(value: &serde_json::Value, paths: &[&[&str]]) -> Option<u64> {
    paths.iter().find_map(|path| read_u64_path(value, path))
}

fn read_u64_path(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_u64()
}

pub(crate) fn normalized_observed_prompt_text(text: &str) -> Option<String> {
    let normalized = strip_observed_attachment_markup(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_observed_attachment_markup(text: &str) -> String {
    ["image", "file"]
        .into_iter()
        .fold(text.to_string(), |current, tag| {
            strip_observed_attachment_tag_blocks(&current, tag)
        })
}

fn strip_observed_attachment_tag_blocks(text: &str, tag: &str) -> String {
    let open_prefix = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find(&open_prefix) {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start..];
        let Some(end) = after_open.find(&close) else {
            output.push_str(after_open);
            return output;
        };
        remaining = &after_open[end + close.len()..];
    }
    output.push_str(remaining);
    output
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
        assert!(
            ExternalProviderObservationPolicy::for_provider(" Codex ").uses_explicit_completion()
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
            assert_eq!(
                policy
                    .observation_for_turn(&ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Status,
                        text: text.to_string(),
                        provider_turn_id: None,
                        observed_at_ms: None,
                    })
                    .map(|observation| observation.settles_active_prompt),
                Some(true),
                "{provider} status should be marked as settling"
            );
            assert_eq!(
                policy
                    .observation_for_turn(&ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Status,
                        text: text.to_string(),
                        provider_turn_id: None,
                        observed_at_ms: None,
                    })
                    .map(|observation| observation.passive_telemetry),
                Some(false),
                "{provider} settling status should not be passive telemetry"
            );
        }
    }

    #[test]
    fn completion_statuses_are_scoped_to_provider_policy() {
        for (provider, foreign_text) in [
            ("codex", "claude message completed\n{}"),
            ("codex", "opencode message completed\n{}"),
            ("claude", "codex task_complete\n{}"),
            (
                "claude",
                "codex event turn_aborted {\"type\":\"turn_aborted\"}",
            ),
            ("claude", "opencode message completed\n{}"),
            ("opencode", "codex task_complete\n{}"),
            ("opencode", "claude message completed\n{}"),
        ] {
            assert!(
                !ExternalProviderObservationPolicy::for_provider(provider)
                    .latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Status,
                        text: foreign_text.to_string(),
                        provider_turn_id: None,
                        observed_at_ms: None,
                    }]),
                "{provider} policy must not settle from foreign marker {foreign_text:?}"
            );
        }
    }

    #[test]
    fn provider_policy_tolerates_legacy_provider_casing_and_whitespace() {
        let codex = ExternalProviderObservationPolicy::for_provider(" Codex ");
        assert!(codex.status_settles("codex task_complete\n{}"));
        assert!(codex.status_is_passive_telemetry("codex token_count\n{}"));

        let claude = ExternalProviderObservationPolicy::for_provider(" CLAUDE ");
        assert!(claude.status_settles("claude message completed\n{}"));
        assert!(
            claude.status_is_passive_telemetry("claude last-prompt {\"lastPrompt\":\"prompt\"}")
        );

        let opencode = ExternalProviderObservationPolicy::for_provider(" OpenCode ");
        assert!(opencode.status_settles("opencode message completed\n{}"));
    }

    #[test]
    fn codex_token_count_status_projects_provider_run_usage() {
        assert_eq!(
            ExternalProviderObservationPolicy::for_provider("codex").status_usage(
                "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42000},\"model_context_window\":128000}}"
            ),
            Some(ProviderRunTokenUsage {
                total_tokens: Some(42_000),
                last_tokens: Some(42_000),
                context_tokens: Some(42_000),
                context_window: Some(128_000),
            })
        );
        assert_eq!(
            ExternalProviderObservationPolicy::for_provider("codex").status_usage(
                "codex token_count\n{\"last\":{\"totalTokens\":160000},\"modelContextWindow\":128000}"
            ),
            Some(ProviderRunTokenUsage {
                total_tokens: Some(160_000),
                last_tokens: Some(160_000),
                context_tokens: None,
                context_window: Some(128_000),
            })
        );
        assert_eq!(
            ExternalProviderObservationPolicy::for_provider("claude")
                .status_usage("codex token_count\n{\"last\":{\"totalTokens\":42}}"),
            None
        );
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
        assert_eq!(
            policy
                .observation_for_turn(&ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                })
                .map(|observation| observation.passive_telemetry),
            Some(true)
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

    #[test]
    fn codex_token_count_is_passive_telemetry_and_does_not_settle() {
        let policy = ExternalProviderObservationPolicy::for_provider("codex");
        let token_count = ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
                .to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        };

        assert!(policy.turn_is_passive_telemetry(&token_count));
        assert!(!policy.latest_effective_turn_settles(std::slice::from_ref(&token_count)));
        assert_eq!(
            policy
                .observation_for_turn(&token_count)
                .map(|observation| observation.passive_telemetry),
            Some(true)
        );
    }

    #[test]
    fn normalized_observed_prompt_text_collapses_whitespace_and_ignores_empty() {
        assert_eq!(
            normalized_observed_prompt_text("  run   this\nnow\t"),
            Some("run this now".to_string())
        );
        assert_eq!(normalized_observed_prompt_text(" \n\t "), None);
    }

    #[test]
    fn normalized_observed_prompt_text_ignores_generated_attachment_markup() {
        assert_eq!(
            normalized_observed_prompt_text(
                "inspect this\n<image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>"
            ),
            Some("inspect this".to_string())
        );
        assert_eq!(
            normalized_observed_prompt_text(
                "read this <file name=\"notes.txt\" path=\"/tmp/notes.txt\"> </file> now"
            ),
            Some("read this now".to_string())
        );
    }

    #[test]
    fn active_external_prompt_turn_uses_latest_user_until_explicit_settlement() {
        let policy = ExternalProviderObservationPolicy::for_provider("codex");
        let turns = vec![
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::User,
                text: "first prompt".to_string(),
                provider_turn_id: Some("user-1".to_string()),
                observed_at_ms: None,
            },
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "working".to_string(),
                provider_turn_id: Some("assistant-1".to_string()),
                observed_at_ms: None,
            },
        ];

        let latest = policy
            .active_external_prompt_turn(&turns, false, &BTreeSet::new())
            .expect("codex should stay active before explicit completion");

        assert_eq!(latest.provider_turn_id.as_deref(), Some("user-1"));
        let settled = [
            turns,
            vec![ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "codex task_complete {}".to_string(),
                provider_turn_id: Some("done".to_string()),
                observed_at_ms: None,
            }],
        ]
        .concat();
        assert!(policy
            .active_external_prompt_turn(&settled, false, &BTreeSet::new())
            .is_none());
    }

    #[test]
    fn active_external_prompt_turn_filters_arroba_owned_provider_turn_ids() {
        let policy = ExternalProviderObservationPolicy::for_provider("claude");
        let mut arroba_owned = BTreeSet::new();
        arroba_owned.insert("user-1".to_string());

        assert!(policy
            .active_external_prompt_turn(
                &[ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::User,
                    text: "same   prompt".to_string(),
                    provider_turn_id: Some("user-1".to_string()),
                    observed_at_ms: None,
                }],
                true,
                &arroba_owned,
            )
            .is_none());
    }

    #[test]
    fn active_prompt_sync_settles_claude_stable_assistant_after_quiet_poll() {
        let policy = ExternalProviderObservationPolicy::for_provider("claude");
        let turns = vec![
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::User,
                text: "prompt".to_string(),
                provider_turn_id: Some("user-1".to_string()),
                observed_at_ms: None,
            },
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "final answer".to_string(),
                provider_turn_id: Some("assistant-1".to_string()),
                observed_at_ms: None,
            },
        ];

        let first_poll = policy.active_prompt_sync(&turns, 2, 2, true, &BTreeSet::new());
        assert!(first_poll.should_sync_active_prompt);
        assert_eq!(
            first_poll
                .active_prompt_turn
                .and_then(|turn| turn.provider_turn_id.as_deref()),
            Some("user-1")
        );

        let quiet_poll = policy.active_prompt_sync(&turns, 0, 0, true, &BTreeSet::new());
        assert!(quiet_poll.should_sync_active_prompt);
        assert!(quiet_poll.active_prompt_turn.is_none());
        assert!(!quiet_poll.latest_observation_settles);
    }

    #[test]
    fn active_prompt_sync_keeps_codex_active_until_explicit_completion() {
        let policy = ExternalProviderObservationPolicy::for_provider("codex");
        let turns = vec![
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::User,
                text: "prompt".to_string(),
                provider_turn_id: Some("user-1".to_string()),
                observed_at_ms: None,
            },
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "intermediate answer".to_string(),
                provider_turn_id: Some("assistant-1".to_string()),
                observed_at_ms: None,
            },
        ];

        let quiet_poll = policy.active_prompt_sync(&turns, 0, 0, true, &BTreeSet::new());
        assert!(quiet_poll.should_sync_active_prompt);
        assert_eq!(
            quiet_poll
                .active_prompt_turn
                .and_then(|turn| turn.provider_turn_id.as_deref()),
            Some("user-1")
        );
        assert!(!quiet_poll.latest_observation_settles);

        let completed_turns = [
            turns,
            vec![ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "codex task_complete\n{}".to_string(),
                provider_turn_id: Some("task-complete-1".to_string()),
                observed_at_ms: None,
            }],
        ]
        .concat();
        let completion = policy.active_prompt_sync(&completed_turns, 1, 1, true, &BTreeSet::new());
        assert!(completion.should_sync_active_prompt);
        assert!(completion.active_prompt_turn.is_none());
        assert!(completion.latest_observation_settles);
    }

    #[test]
    fn active_prompt_sync_ignores_passive_telemetry_only_poll() {
        let policy = ExternalProviderObservationPolicy::for_provider("codex");
        let turns = vec![ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
                .to_string(),
            provider_turn_id: Some("usage-1".to_string()),
            observed_at_ms: None,
        }];

        let sync = policy.active_prompt_sync(&turns, 1, 0, true, &BTreeSet::new());
        assert!(!sync.should_sync_active_prompt);
        assert!(sync.active_prompt_turn.is_none());
        assert!(!sync.latest_observation_settles);
    }
}
