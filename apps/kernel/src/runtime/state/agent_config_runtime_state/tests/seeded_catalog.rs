//! Tests that drive a fake provider process through a profile update seed the model list the update
//! reads, so no catalog probe reaches the fake.
use super::*;
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
use std::collections::BTreeMap;

/// Puts a Claude catalog with the `haiku` and `sonnet` aliases the fixtures run into the projection
/// for the owner's default Claude account, under the provider name the agent runs.
pub(super) async fn seed_claude_aliases(app: &Arc<Mutex<DaemonApp>>, provider: &str) {
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let models = ["haiku", "sonnet"].map(|id| {
        (
            id.to_string(),
            OpenCodeProviderModel {
                id: id.to_string(),
                name: id.to_string(),
                status: "active".to_string(),
                limit: None,
                variants: BTreeMap::new(),
            },
        )
    });
    let app = app.lock().await;
    let profile = app
        .provider_account_profile_registry()
        .get(owner, "claude", "default")
        .unwrap()
        .profile_id;
    let request = crate::local::GetProviderCatalogRequest {
        provider: Some(provider.to_string()),
        account_profiles: BTreeMap::from([("claude".to_string(), profile)]),
        ..Default::default()
    };
    let key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(owner, &request);
    app.provider_catalog_projection_store().update_scoped(
        &key,
        OpenCodeProviderCatalog {
            all: vec![OpenCodeProviderInfo {
                id: provider.to_string(),
                name: provider.to_string(),
                remote_machine_aliases: Vec::new(),
                models: models.into_iter().collect(),
            }],
            default: BTreeMap::from([(provider.to_string(), "sonnet".to_string())]),
            connected: vec![provider.to_string()],
        },
    );
}
