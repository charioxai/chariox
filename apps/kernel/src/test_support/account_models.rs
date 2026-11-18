//! Lists the models of a provider account in the kernel's catalog projection, as a live read of
//! its catalog would, so a test that names a model needs no installed provider.
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
use crate::runtime::projection::ProviderCatalogProjectionStore;
use std::collections::BTreeMap;

/// A listed model has no variants, so a variant a test names is kept and none is invented.
/// `models` are the ids the account offers; an OpenCode id is `backend/model`.
pub(crate) fn seed_account_models(
    projection: &ProviderCatalogProjectionStore,
    owner_user_id: &str,
    provider_name: &str,
    profile_id: &str,
    models: &[&str],
) {
    let family = crate::provider::canonical_provider_family(provider_name)
        .expect("a provider with a catalog");
    let mut providers: BTreeMap<String, OpenCodeProviderInfo> = BTreeMap::new();
    for id in models {
        let (provider_id, model_id) = match (family, id.split_once('/')) {
            ("opencode", Some((backend, model))) => (backend.to_string(), model),
            ("opencode", None) => ("opencode-go".to_string(), *id),
            ("codex", _) => ("codex".to_string(), *id),
            _ if provider_name == "claude-headless" => ("claude-headless".to_string(), *id),
            _ => ("claude-p".to_string(), *id),
        };
        providers
            .entry(provider_id.clone())
            .or_insert_with(|| OpenCodeProviderInfo {
                id: provider_id,
                name: String::new(),
                remote_machine_aliases: Vec::new(),
                models: BTreeMap::new(),
            })
            .models
            .insert(
                model_id.to_string(),
                OpenCodeProviderModel {
                    id: model_id.to_string(),
                    name: model_id.to_string(),
                    status: "active".to_string(),
                    limit: None,
                    variants: BTreeMap::new(),
                },
            );
    }
    let connected = providers.keys().cloned().collect();
    let catalog = OpenCodeProviderCatalog {
        all: providers
            .into_values()
            .map(|mut info| {
                info.name = info.id.clone();
                info
            })
            .collect(),
        default: BTreeMap::new(),
        connected,
    };
    let request = crate::local::GetProviderCatalogRequest {
        provider: Some(provider_name.to_string()),
        account_profiles: BTreeMap::from([(family.to_string(), profile_id.to_string())]),
        ..Default::default()
    };
    projection.update_scoped(
        &crate::runtime::provider_catalog_control::provider_catalog_cache_key(
            owner_user_id,
            &request,
        ),
        catalog,
    );
}

/// The same for the local owner's default account of `provider_name`.
pub(crate) fn seed_default_account_models(
    app: &crate::DaemonApp,
    provider_name: &str,
    models: &[&str],
) {
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let profile = app
        .provider_account_profile_registry()
        .get(owner, provider_name, "default")
        .expect("the default account");
    seed_account_models(
        &app.provider_catalog_projection_store(),
        owner,
        provider_name,
        &profile.profile_id,
        models,
    );
}
