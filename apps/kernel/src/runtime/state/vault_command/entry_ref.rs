//! One name-to-id step for every Vault tool that takes an entry
//! (`use_env`, `write_file`, `copy_for_use`): the exact credential_id wins;
//! otherwise a description that names exactly one entry; an ambiguous name is
//! refused and lists the candidate ids. A name that matches nothing passes
//! through unchanged, so each tool keeps its own "no entry" refusal.
use super::*;
use crate::config::UserCredentialConfig;

pub(super) fn resolve_entry_ref(
    entries: &[UserCredentialConfig],
    reference: &str,
) -> Result<String, DaemonError> {
    if entries.iter().any(|entry| entry.id == reference) {
        return Ok(reference.to_string());
    }
    let needle = reference.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(reference.to_string());
    }
    let named: Vec<&str> = entries
        .iter()
        .filter(|entry| {
            entry
                .description
                .as_deref()
                .is_some_and(|d| d.trim().to_lowercase() == needle)
        })
        .map(|entry| entry.id.as_str())
        .collect();
    match named.as_slice() {
        [] => Ok(reference.to_string()),
        [id] => Ok((*id).to_string()),
        ids => Err(error(format!(
            "more than one Vault entry is described as `{}`: {}; name one by credential_id",
            reference.trim(),
            ids.join(", ")
        ))),
    }
}

impl KernelRuntimeState {
    /// Resolve every reference against the owner's Vault entries once.
    pub(super) fn resolve_entry_refs<'a>(
        &self,
        references: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<String>, DaemonError> {
        let entries = crate::credential::CharioxCredentialRegistry::user()?.list()?;
        references
            .into_iter()
            .map(|reference| resolve_entry_ref(&entries, reference))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{UserCredentialInjectionConfig, UserCredentialSourceConfig};

    fn entry(id: &str, description: Option<&str>) -> UserCredentialConfig {
        UserCredentialConfig {
            id: id.into(),
            description: description.map(str::to_string),
            source: UserCredentialSourceConfig::Vault { key: id.into() },
            allowed_hosts: vec![],
            allowed_uses: vec![],
            injection: UserCredentialInjectionConfig::Env,
            metadata: None,
        }
    }

    #[test]
    fn the_exact_id_wins_even_when_another_entry_is_described_with_it() {
        let entries = [
            entry("gen-1", Some("gen-2")),
            entry("gen-2", Some("webhook")),
        ];
        assert_eq!(resolve_entry_ref(&entries, "gen-2").unwrap(), "gen-2");
    }

    #[test]
    fn a_unique_description_names_its_entry_ignoring_case_and_edges() {
        let entries = [
            entry("a1", Some("Webhook signing secret")),
            entry("b2", Some("deploy key")),
        ];
        assert_eq!(
            resolve_entry_ref(&entries, "  webhook SIGNING secret ").unwrap(),
            "a1"
        );
    }

    #[test]
    fn an_ambiguous_description_is_refused_and_lists_the_ids() {
        let entries = [
            entry("a1", Some("deploy key")),
            entry("b2", Some("Deploy key")),
        ];
        let refused = resolve_entry_ref(&entries, "deploy key")
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a1") && refused.contains("b2"),
            "{refused}"
        );
        assert!(refused.contains("more than one"), "{refused}");
    }

    #[test]
    fn a_name_that_matches_nothing_passes_through_for_the_tool_to_refuse() {
        let entries = [entry("a1", Some("deploy key")), entry("b2", None)];
        assert_eq!(resolve_entry_ref(&entries, "nope").unwrap(), "nope");
        assert_eq!(resolve_entry_ref(&entries, "").unwrap(), "");
    }

    #[test]
    fn an_entry_without_a_description_is_only_reachable_by_id() {
        let entries = [entry("b2", None)];
        assert_eq!(resolve_entry_ref(&entries, "b2").unwrap(), "b2");
    }
}
