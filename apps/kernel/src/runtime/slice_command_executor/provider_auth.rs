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
    match provider {
        "all" | "codex" | "opencode" | "claude" => Ok(provider.to_string()),
        "claude-headless" | "claude-p" => Ok("claude".to_string()),
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
    use super::normalized_slice_provider;

    #[test]
    fn normalizes_claude_provider_modes_to_slice_auth_provider() {
        assert_eq!(
            normalized_slice_provider("claude-headless").unwrap(),
            "claude"
        );
        assert_eq!(normalized_slice_provider("claude-p").unwrap(), "claude");
    }
}
