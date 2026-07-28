use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{now_ms, AegsStore, ProviderHookRecord};

pub fn reconcile_provider_hooks<Create, Delete, Refresh>(
    store: &AegsStore,
    generator_id: &str,
    create: Create,
    delete: Delete,
    refresh: Refresh,
) -> Result<(), String>
where
    Create: Fn(&str, &str, &BTreeSet<String>, &str) -> Result<ProviderHookRecord, String>,
    Delete: Fn(&ProviderHookRecord) -> Result<(), String>,
    Refresh: Fn(&ProviderHookRecord) -> Result<Option<ProviderHookRecord>, String>,
{
    let claims = store.all(generator_id)?;
    let mut desired = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for claim in claims.iter().filter(|claim| claim.active) {
        desired
            .entry((claim.connection_id.clone(), claim.connection_scope.clone()))
            .or_default()
            .insert(claim.event_type.clone());
    }
    let existing = store
        .provider_hooks()?
        .into_iter()
        .map(|hook| {
            (
                (hook.connection_id.clone(), hook.connection_scope.clone()),
                hook,
            )
        })
        .collect::<BTreeMap<_, _>>();

    for (key, hook) in &existing {
        if !desired.contains_key(key) {
            delete(hook)?;
            store.delete_provider_hook(&hook.connection_id, &hook.connection_scope)?;
        }
    }
    for ((connection_id, connection_scope), event_types) in desired {
        let configuration_digest = event_configuration_digest(&event_types);
        if let Some(current) = existing.get(&(connection_id.clone(), connection_scope.clone())) {
            if current.configuration_digest == configuration_digest {
                if let Some(renewed) = refresh(current)? {
                    store.upsert_provider_hook(&renewed, now_ms())?;
                }
                continue;
            }
            delete(current)?;
        }
        let hook = create(
            &connection_id,
            &connection_scope,
            &event_types,
            &configuration_digest,
        )?;
        store.upsert_provider_hook(&hook, now_ms())?;
    }
    Ok(())
}

pub fn event_configuration_digest(event_types: &BTreeSet<String>) -> String {
    let mut hasher = Sha256::new();
    for event_type in event_types {
        hasher.update((event_type.len() as u64).to_be_bytes());
        hasher.update(event_type.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_order_independent_for_sorted_event_set() {
        let left = BTreeSet::from(["issue.created".to_string(), "issue.updated".to_string()]);
        let right = BTreeSet::from(["issue.updated".to_string(), "issue.created".to_string()]);
        assert_eq!(
            event_configuration_digest(&left),
            event_configuration_digest(&right)
        );
    }
}
