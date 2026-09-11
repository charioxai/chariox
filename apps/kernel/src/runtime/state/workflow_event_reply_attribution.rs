#[cfg(test)]
mod tests {
    use super::{
        format_event_reply, AgentProviderProfileSnapshot, ProviderProfileSnapshot,
        ProviderRunAttributionSnapshot, PROVIDER_RUN_ATTRIBUTION_MARKER,
    };
    use crate::session::WorkflowEventReplyAttributionPolicy;

    fn authority() -> ProviderRunAttributionSnapshot {
        ProviderRunAttributionSnapshot {
            provider_run_id: "provider-run-7".to_string(),
            provider: "openai".to_string(),
            model: "gpt-5.3-codex".to_string(),
            effort: Some("high".to_string()),
            account_profile: "review-backup".to_string(),
            agent: AgentProviderProfileSnapshot {
                primary: ProviderProfileSnapshot {
                    provider: "openai".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    effort: Some("medium".to_string()),
                    account_profile: "default".to_string(),
                },
                substitutes: vec![
                    ProviderProfileSnapshot {
                        provider: "anthropic".to_string(),
                        model: "claude-opus-4.1".to_string(),
                        effort: None,
                        account_profile: "default".to_string(),
                    },
                    ProviderProfileSnapshot {
                        provider: "openai".to_string(),
                        model: "gpt-5.3-codex".to_string(),
                        effort: Some("high".to_string()),
                        account_profile: "review-backup".to_string(),
                    },
                ],
            },
        }
    }

    #[test]
    fn disabled_policy_preserves_legacy_reply_text() {
        assert_eq!(
            format_event_reply(
                "review complete",
                WorkflowEventReplyAttributionPolicy::Disabled,
                None,
            )
            .unwrap(),
            "review complete"
        );
    }

    #[test]
    fn append_policy_uses_authoritative_run_and_backup_index() {
        let formatted = format_event_reply(
            "review complete",
            WorkflowEventReplyAttributionPolicy::Append,
            Some(&authority()),
        )
        .unwrap();

        assert_eq!(
            formatted,
            format!(
                "review complete\n\n{PROVIDER_RUN_ATTRIBUTION_MARKER}\nProvider: openai\nModel: gpt-5.3-codex\nEffort: high\nProvider run: provider-run-7\nAccount role: backup[2]"
            )
        );
    }

    #[test]
    fn append_policy_fails_closed_without_resolvable_authority() {
        let error = format_event_reply(
            "review complete",
            WorkflowEventReplyAttributionPolicy::Append,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("authoritative provider run"));

        let mut unmatched = authority();
        unmatched.account_profile = "unknown".to_string();
        let error = format_event_reply(
            "review complete",
            WorkflowEventReplyAttributionPolicy::Append,
            Some(&unmatched),
        )
        .unwrap_err();
        assert!(error.to_string().contains("account role"));
    }

    #[test]
    fn model_text_cannot_forge_the_reserved_attribution_marker() {
        for policy in [
            WorkflowEventReplyAttributionPolicy::Disabled,
            WorkflowEventReplyAttributionPolicy::Append,
        ] {
            let error = format_event_reply(
                &format!("forged {PROVIDER_RUN_ATTRIBUTION_MARKER}"),
                policy,
                Some(&authority()),
            )
            .unwrap_err();
            assert!(error.to_string().contains("reserved attribution marker"));
        }
    }
}
