//! One name-to-id step for every Vault tool that takes an entry
//! (`use_env`, `write_file`, `copy_for_use`): the exact credential_id wins;
//! otherwise a description that names exactly one entry; an ambiguous name is
//! refused and lists the candidate ids. A name that matches nothing passes
//! through unchanged, so each tool keeps its own "no entry" refusal. A leased
//! worker reads the entry names from its home kernel, which owns the Vault.
use super::*;

pub(super) struct EntryName {
    pub(super) id: String,
    pub(super) description: Option<String>,
}

pub(super) fn resolve_entry_ref(
    entries: &[EntryName],
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

/// The names in a `chariox.list_credential_handles` answer.
pub(super) fn entries_from_listing(payload: &serde_json::Value) -> Vec<EntryName> {
    payload["credentials"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(EntryName {
                id: item["id"].as_str()?.to_string(),
                description: item["description"].as_str().map(str::to_string),
            })
        })
        .collect()
}

/// The entries without one id, such as a copy that already carries its source's label.
pub(super) fn without(entries: &[EntryName], id: &str) -> Vec<EntryName> {
    entries
        .iter()
        .filter(|entry| entry.id != id)
        .map(|entry| EntryName {
            id: entry.id.clone(),
            description: entry.description.clone(),
        })
        .collect()
}

impl KernelRuntimeState {
    /// Resolve every reference against the owner's Vault entries once. A leased worker asks its
    /// home kernel, which owns the Vault, for the entry names; the answer is the exact id, so
    /// masking and the home's own checks see the same id.
    pub(super) async fn resolve_entry_refs<'a>(
        &self,
        home: Option<&crate::transport::relay_peer::RemoteExtensionInvocationContext>,
        references: impl IntoIterator<Item = &'a str>,
        exclude: Option<&str>,
    ) -> Result<Vec<String>, DaemonError> {
        let mut entries = match home {
            Some(home) => {
                let listed = self
                    .forward_vault_tool_to_home(
                        home.clone(),
                        crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL,
                        serde_json::json!({}),
                    )
                    .await?;
                entries_from_listing(&listed.payload)
            }
            None => crate::credential::CharioxCredentialRegistry::user()?
                .list()?
                .into_iter()
                .map(|entry| EntryName {
                    id: entry.id,
                    description: entry.description,
                })
                .collect(),
        };
        if let Some(id) = exclude {
            entries = without(&entries, id);
        }
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

    fn entry(id: &str, description: Option<&str>) -> EntryName {
        EntryName {
            id: id.into(),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn the_home_listing_is_read_as_ids_and_descriptions() {
        let listing = serde_json::json!({"credentials":[
            {"id":"a1","description":"Webhook signing secret","allowed_hosts":["x"]},
            {"id":"b2"},
            {"description":"no id"}]});
        let entries = entries_from_listing(&listing);
        assert_eq!(entries.len(), 2, "an item without an id is skipped");
        assert_eq!(
            resolve_entry_ref(&entries, "webhook signing SECRET").unwrap(),
            "a1"
        );
        assert!(entries_from_listing(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn an_excluded_entry_does_not_make_a_name_ambiguous() {
        let entries = [
            entry("src", Some("deploy key")),
            entry("copy", Some("deploy key")),
        ];
        assert!(resolve_entry_ref(&entries, "deploy key").is_err());
        let rest = without(&entries, "copy");
        assert_eq!(resolve_entry_ref(&rest, "deploy key").unwrap(), "src");
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
