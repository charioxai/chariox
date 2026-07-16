use crate::error::DaemonError;
use crate::slice_provider_auth::SliceProviderAuthSummary;

pub(super) fn normalized_slice_provider(provider: &str) -> Result<String, DaemonError> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.provider",
            message: "provider must not be empty".to_string(),
        });
    }
    if provider == "all" {
        return Ok(provider.to_string());
    }
    if provider == "github" {
        return Ok(provider.to_string());
    }
    if let Some(provider_family) = crate::provider::canonical_provider_family(provider) {
        return Ok(provider_family.to_string());
    }
    match provider {
        value if value.starts_with("opencode:") && value.len() > "opencode:".len() => {
            Ok(value.to_string())
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "slice.auth.provider",
            message: format!("unsupported slice provider `{provider}`"),
        }),
    }
}

pub(super) fn scoped_provider_auth_summaries(
    provider: &str,
    summaries: Vec<SliceProviderAuthSummary>,
) -> Vec<SliceProviderAuthSummary> {
    summaries
        .into_iter()
        .filter(|summary| slice_auth_summary_matches_provider(&summary.provider, provider))
        .collect()
}

pub(super) fn merge_scoped_provider_auth(
    existing: Vec<SliceProviderAuthSummary>,
    provider: &str,
    imported: Vec<SliceProviderAuthSummary>,
) -> Vec<SliceProviderAuthSummary> {
    let aliases = existing
        .iter()
        .filter(|summary| slice_auth_summary_matches_provider(&summary.provider, provider))
        .filter_map(|summary| {
            summary
                .alias
                .as_ref()
                .map(|alias| (summary.provider.clone(), alias.clone()))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut merged = existing
        .into_iter()
        .filter(|summary| !slice_auth_summary_matches_provider(&summary.provider, provider))
        .collect::<Vec<_>>();
    merged.extend(imported.into_iter().map(|mut summary| {
        if summary.alias.is_none() {
            summary.alias = aliases.get(&summary.provider).cloned();
        }
        summary
    }));
    merged
}

pub(super) fn merge_detected_provider_auth(
    existing: Vec<SliceProviderAuthSummary>,
    detected: Vec<SliceProviderAuthSummary>,
) -> Vec<SliceProviderAuthSummary> {
    let failed_providers = existing
        .iter()
        .filter(|summary| summary.source == "provider_auth_failure")
        .map(|summary| summary.provider.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let detected = detected.into_iter().filter(|summary| {
        !failed_providers.contains(&summary.provider)
            || summary.source != "slice_provider_auth_file"
    });
    crate::slice_provider_auth::merge_provider_auth_summaries(
        existing.into_iter().chain(detected).collect(),
    )
}

pub(super) fn slice_auth_summary_matches_provider(
    summary_provider: &str,
    requested_provider: &str,
) -> bool {
    if requested_provider == "all" {
        return true;
    }
    if requested_provider == "opencode" {
        return summary_provider == "opencode" || summary_provider.starts_with("opencode:");
    }
    summary_provider == requested_provider
}

#[cfg(test)]
mod tests {
    use crate::slice_provider_auth::{SliceProviderAuthState, SliceProviderAuthSummary};

    use super::{merge_detected_provider_auth, normalized_slice_provider};

    #[test]
    fn normalizes_claude_provider_modes_to_slice_auth_provider() {
        assert_eq!(
            normalized_slice_provider("claude-headless").unwrap(),
            "claude"
        );
        assert_eq!(normalized_slice_provider("claude-p").unwrap(), "claude");
        assert_eq!(normalized_slice_provider("github").unwrap(), "github");
    }

    #[test]
    fn detected_slice_auth_adds_new_provider_without_losing_existing_metadata() {
        let existing = SliceProviderAuthSummary {
            provider: "claude".to_string(),
            state: SliceProviderAuthState::Authenticated,
            auth_type: Some("claude.ai".to_string()),
            account_id: Some("claude-account".to_string()),
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: Some("Work Claude".to_string()),
            source: "existing".to_string(),
        };
        let detected = SliceProviderAuthSummary {
            provider: "opencode".to_string(),
            state: SliceProviderAuthState::Configured,
            auth_type: None,
            account_id: None,
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: None,
            source: "slice_provider_auth_file".to_string(),
        };

        let merged = merge_detected_provider_auth(vec![existing], vec![detected]);

        assert_eq!(merged.len(), 2);
        let claude = merged
            .iter()
            .find(|summary| summary.provider == "claude")
            .unwrap();
        assert_eq!(claude.alias.as_deref(), Some("Work Claude"));
        assert!(merged.iter().any(|summary| summary.provider == "opencode"));
    }

    #[test]
    fn auth_file_presence_does_not_override_a_runtime_auth_failure() {
        let failed = SliceProviderAuthSummary {
            provider: "codex".to_string(),
            state: SliceProviderAuthState::NotConfigured,
            auth_type: None,
            account_id: None,
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: Some("Work Codex".to_string()),
            source: "provider_auth_failure".to_string(),
        };
        let detected = SliceProviderAuthSummary {
            provider: "codex".to_string(),
            state: SliceProviderAuthState::Configured,
            auth_type: None,
            account_id: None,
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: None,
            source: "slice_provider_auth_file".to_string(),
        };

        let merged = merge_detected_provider_auth(vec![failed], vec![detected]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].state, SliceProviderAuthState::NotConfigured);
        assert_eq!(merged[0].alias.as_deref(), Some("Work Codex"));
    }
}
