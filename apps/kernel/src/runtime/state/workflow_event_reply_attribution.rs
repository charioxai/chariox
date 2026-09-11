use crate::error::DaemonError;
use crate::session::WorkflowEventReplyAttributionPolicy;

pub(super) const PROVIDER_RUN_ATTRIBUTION_MARKER: &str =
    "<!-- chariox:provider-run-attribution:v1 -->";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProviderProfileSnapshot {
    pub provider: String,
    pub model: String,
    pub effort: Option<String>,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentProviderProfileSnapshot {
    pub primary: ProviderProfileSnapshot,
    pub substitutes: Vec<ProviderProfileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProviderRunAttributionSnapshot {
    pub provider_run_id: String,
    pub provider: String,
    pub model: String,
    pub effort: Option<String>,
    pub account_profile: String,
    pub agent: AgentProviderProfileSnapshot,
}

impl ProviderRunAttributionSnapshot {
    pub(super) fn capture(
        run: &crate::provider::RuntimeProviderRun,
        agent: &crate::agent::AgentInstance,
    ) -> Self {
        Self {
            provider_run_id: run.id().to_string(),
            provider: run.provider().to_string(),
            model: run.model().to_string(),
            effort: run.variant().map(str::to_string),
            account_profile: run.account_profile().to_string(),
            agent: AgentProviderProfileSnapshot {
                primary: ProviderProfileSnapshot {
                    provider: agent.primary_provider().to_string(),
                    model: agent.primary_model().unwrap_or_default().to_string(),
                    effort: agent.primary_effort().map(str::to_string),
                    account_profile: agent
                        .primary_account_profile()
                        .unwrap_or("default")
                        .to_string(),
                },
                substitutes: agent
                    .substitutes()
                    .iter()
                    .map(|profile| ProviderProfileSnapshot {
                        provider: profile.provider.clone(),
                        model: profile.model.clone(),
                        effort: profile.variant.clone(),
                        account_profile: profile
                            .account_profile
                            .as_deref()
                            .unwrap_or("default")
                            .to_string(),
                    })
                    .collect(),
            },
        }
    }

    fn account_role(&self) -> Option<String> {
        let run_profile = ProviderProfileSnapshot {
            provider: self.provider.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            account_profile: self.account_profile.clone(),
        };
        if run_profile == self.agent.primary {
            return Some("primary".to_string());
        }
        self.agent
            .substitutes
            .iter()
            .position(|profile| profile == &run_profile)
            .map(|index| format!("backup[{}]", index + 1))
    }
}

pub(super) fn format_event_reply(
    text: &str,
    policy: WorkflowEventReplyAttributionPolicy,
    authority: Option<&ProviderRunAttributionSnapshot>,
) -> Result<String, DaemonError> {
    if text.contains(PROVIDER_RUN_ATTRIBUTION_MARKER) {
        return Err(attribution_error(
            "reply text contains the reserved attribution marker",
        ));
    }
    if policy == WorkflowEventReplyAttributionPolicy::Disabled {
        return Ok(text.to_string());
    }
    let authority = authority.ok_or_else(|| {
        attribution_error("authoritative provider run attribution could not be resolved")
    })?;
    for (name, value) in [
        ("provider", authority.provider.as_str()),
        ("model", authority.model.as_str()),
        ("provider run ID", authority.provider_run_id.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(attribution_error(format!(
                "authoritative {name} is not safe to format"
            )));
        }
    }
    let effort = authority.effort.as_deref().unwrap_or("default");
    if effort.chars().any(char::is_control) {
        return Err(attribution_error(
            "authoritative effort is not safe to format",
        ));
    }
    let account_role = authority.account_role().ok_or_else(|| {
        attribution_error("authoritative provider run account role could not be resolved")
    })?;
    let formatted = format!(
        "{text}\n\n{PROVIDER_RUN_ATTRIBUTION_MARKER}\nProvider: {}\nModel: {}\nEffort: {effort}\nProvider run: {}\nAccount role: {account_role}",
        authority.provider, authority.model, authority.provider_run_id
    );
    if formatted.len() > 40_000 {
        return Err(attribution_error(
            "attributed reply exceeds the 40000 character limit",
        ));
    }
    Ok(formatted)
}

fn attribution_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "format event reply attribution",
        message: message.into(),
    }
}

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
