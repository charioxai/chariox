use crate::error::DaemonError;
use crate::local::provider_requests::{
    load_provider_catalog, provider_command_catalogs_response, PROVIDER_CATALOG_CACHE_TTL,
};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use std::time::Instant;

pub(crate) async fn execute_provider_catalog_request(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderCatalog(request) => {
            execute_get_provider_catalog_request(
                provider_catalog_projection,
                config_projection,
                account_profiles,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetClientCatalogSnapshot(request) => {
            super::client_catalog_revisions::catalog_snapshot(
                provider_catalog_projection,
                config_projection,
                account_profiles,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetClientCatalogRevisions(request) => {
            super::client_catalog_revisions::catalog_revisions(
                provider_catalog_projection,
                config_projection,
                account_profiles,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
            execute_get_provider_command_catalogs_request()
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider catalog request",
            message: "unsupported provider catalog request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_catalog_request(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: crate::local::GetProviderCatalogRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let catalog = load_catalog(
        provider_catalog_projection,
        config_projection,
        account_profiles,
        owner_user_id,
        request,
        true,
    )
    .await?;
    Ok(LocalDaemonResponse::ProviderCatalog { catalog })
}

/// What a catalog read can honestly answer. `Revalidating` means the scope has no
/// kernel copy yet, the caller already holds one, and discovery runs in the
/// background; a change event follows when it ends.
pub(super) enum CatalogRead {
    Loaded(crate::provider::OpenCodeProviderCatalog),
    Revalidating,
}

// Both ordinary full reads and rare revision checks use the same scoped loader.
// A revision check reconciles stale provider metadata before comparing hashes;
// the client keeps its stored catalog visible while that work runs.
pub(super) async fn load_catalog(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: crate::local::GetProviderCatalogRequest,
    allow_stale: bool,
) -> Result<crate::provider::OpenCodeProviderCatalog, DaemonError> {
    match read_catalog(
        provider_catalog_projection,
        config_projection,
        account_profiles,
        owner_user_id,
        request,
        allow_stale,
        false,
    )
    .await?
    {
        CatalogRead::Loaded(catalog) => Ok(catalog),
        CatalogRead::Revalidating => {
            unreachable!("only a caller holding a catalog may be answered while revalidating")
        }
    }
}

// Model discovery can take many seconds and must never hold back the account,
// command and revision sections of a snapshot the client could already render.
pub(super) async fn read_catalog(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: crate::local::GetProviderCatalogRequest,
    allow_stale: bool,
    caller_holds_catalog: bool,
) -> Result<CatalogRead, DaemonError> {
    // Preserve the admitted caller for explicit replica references. Defaults
    // and home aliases are resolved per provider below the catalog loader.
    let owner_user_id = owner_user_id.to_string();
    let cache_key = provider_catalog_cache_key(&owner_user_id, &request);
    if let Some(catalog) =
        provider_catalog_projection.get_scoped(&cache_key, PROVIDER_CATALOG_CACHE_TTL)
    {
        crate::logging::info_with_fields(
            "daemon.startup",
            "provider catalog cache hit",
            serde_json::json!({
                "provider_count": catalog.all.len(),
                "connected_provider_count": catalog.connected.len(),
            }),
        );
        return Ok(CatalogRead::Loaded(catalog));
    }
    let stale = provider_catalog_projection
        .cached_scoped(&cache_key)
        .filter(|_| allow_stale);
    if stale.is_some() || (allow_stale && caller_holds_catalog) {
        refresh_provider_catalog_in_background(
            provider_catalog_projection.clone(),
            config_projection.snapshot(),
            account_profiles.clone(),
            owner_user_id.clone(),
            request,
            cache_key,
        );
        return Ok(match stale {
            Some(catalog) => {
                crate::logging::info_with_fields(
                    "daemon.startup",
                    "serving stale provider catalog while refresh runs",
                    serde_json::json!({
                        "provider_count": catalog.all.len(),
                        "connected_provider_count": catalog.connected.len(),
                    }),
                );
                CatalogRead::Loaded(catalog)
            }
            None => CatalogRead::Revalidating,
        });
    }

    let _load_guard = provider_catalog_projection.lock_load(&cache_key).await;
    // Another caller may have filled this scope while we waited.
    if let Some(catalog) =
        provider_catalog_projection.get_scoped(&cache_key, PROVIDER_CATALOG_CACHE_TTL)
    {
        return Ok(CatalogRead::Loaded(catalog));
    }
    let generation = provider_catalog_projection.invalidation_generation();
    let config = config_projection.snapshot();
    let registry = account_profiles.clone();
    let load_started = Instant::now();
    let load_owner = owner_user_id.clone();
    let catalog = tokio::task::spawn_blocking(move || {
        load_provider_catalog(config, registry, load_owner, request)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load provider catalog",
        message: error.to_string(),
    })??;
    crate::logging::info_with_fields(
        "daemon.startup",
        "provider catalog loaded",
        serde_json::json!({
            "load_ms": load_started.elapsed().as_millis(),
            "owner_user_id": owner_user_id,
            "provider_count": catalog.all.len(),
            "connected_provider_count": catalog.connected.len(),
        }),
    );
    if !provider_catalog_projection.update_scoped_if_generation(
        &cache_key,
        catalog.clone(),
        generation,
    ) {
        // An account change during discovery must not fail the read.
        provider_catalog_projection.update_scoped_stale_if_absent(
            &cache_key,
            catalog.clone(),
            generation,
        );
    }
    Ok(CatalogRead::Loaded(catalog))
}

fn refresh_provider_catalog_in_background(
    provider_catalog_projection: ProviderCatalogProjectionStore,
    config: crate::config::DaemonConfig,
    registry: crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: String,
    request: crate::local::GetProviderCatalogRequest,
    cache_key: String,
) {
    let Some(generation) = provider_catalog_projection.begin_refresh(&cache_key) else {
        return;
    };
    tokio::spawn(async move {
        let _load_guard = provider_catalog_projection.lock_load(&cache_key).await;
        if provider_catalog_projection
            .get_scoped(&cache_key, PROVIDER_CATALOG_CACHE_TTL)
            .is_some()
        {
            provider_catalog_projection.finish_refresh(&cache_key, generation);
            return;
        }
        let load_started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            load_provider_catalog(config, registry, owner_user_id, request)
        })
        .await;
        match result {
            Ok(Ok(catalog)) => {
                crate::logging::info_with_fields(
                    "daemon.startup",
                    "provider catalog refreshed in background",
                    serde_json::json!({
                        "load_ms": load_started.elapsed().as_millis(),
                        "provider_count": catalog.all.len(),
                        "connected_provider_count": catalog.connected.len(),
                    }),
                );
                if !provider_catalog_projection.update_scoped_if_generation(
                    &cache_key,
                    catalog.clone(),
                    generation,
                ) {
                    provider_catalog_projection
                        .update_scoped_stale_if_absent(&cache_key, catalog, generation);
                    crate::logging::info(
                        "daemon.startup",
                        "provider catalog refresh overtaken by an invalidation; kept only as stale metadata",
                    );
                }
            }
            Ok(Err(error)) => crate::logging::warn_with_fields(
                "daemon.startup",
                "provider catalog background refresh failed",
                serde_json::json!({
                    "load_ms": load_started.elapsed().as_millis(),
                    "error": error.to_string(),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.startup",
                "provider catalog background refresh task failed",
                serde_json::json!({
                    "load_ms": load_started.elapsed().as_millis(),
                    "error": error.to_string(),
                }),
            ),
        }
        provider_catalog_projection.finish_refresh(&cache_key, generation);
    });
}

pub(crate) fn execute_get_provider_command_catalogs_request(
) -> Result<LocalDaemonResponse, DaemonError> {
    provider_command_catalogs_response()
}

pub(crate) async fn provider_catalog_json_value(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
) -> Option<serde_json::Value> {
    if let Some(catalog) = provider_catalog_projection.get(PROVIDER_CATALOG_CACHE_TTL) {
        return serde_json::to_value(catalog).ok();
    }
    let _load_guard = provider_catalog_projection.lock_load("").await;
    if let Some(catalog) = provider_catalog_projection.get(PROVIDER_CATALOG_CACHE_TTL) {
        return serde_json::to_value(catalog).ok();
    }
    let config = config_projection.snapshot();
    let registry = account_profiles.clone();
    let load_started = Instant::now();
    tokio::task::spawn_blocking(move || {
        load_provider_catalog(
            config,
            registry,
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::local::GetProviderCatalogRequest::default(),
        )
    })
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|catalog| {
        crate::logging::info_with_fields(
            "daemon.startup",
            "provider catalog loaded for health projection",
            serde_json::json!({
                "load_ms": load_started.elapsed().as_millis(),
                "provider_count": catalog.all.len(),
                "connected_provider_count": catalog.connected.len(),
            }),
        );
        serde_json::to_value(catalog).ok()
    })
}

pub(crate) fn provider_catalog_cache_key(
    owner_user_id: &str,
    request: &crate::local::GetProviderCatalogRequest,
) -> String {
    format!(
        "{}:{}",
        owner_user_id,
        serde_json::to_string(request).expect("provider catalog request serializes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // MP-08 / MP-10 / MP-11: concurrent fresh clients share one native catalog load.
    #[tokio::test]
    async fn amfix_r18_catalog_coalesces_concurrent_initial_reads() {
        crate::test_support::isolated_env_test!();
        let capture = crate::logging::capture::start();
        let root =
            std::env::temp_dir().join(format!("amfix-catalog-flight-{}", rand::random::<u64>()));
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let owner = format!("amfix-single-flight-{}", rand::random::<u64>());
        let profile = registry
            .create_managed(&owner, "claude", "Catalog")
            .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.provider_catalog_read_delay_ms = 100;
        let config = DaemonConfigProjectionStore::new(config);
        let projection = ProviderCatalogProjectionStore::default();
        let request = crate::local::GetProviderCatalogRequest {
            provider: Some("claude".into()),
            account_profiles: BTreeMap::from([("claude".into(), profile.profile_id)]),
            ..Default::default()
        };
        let reads = (0..6).map(|_| {
            execute_get_provider_catalog_request(
                &projection,
                &config,
                &registry,
                &owner,
                request.clone(),
            )
        });
        for result in futures_util::future::join_all(reads).await {
            assert!(result.is_ok(), "{result:?}");
        }
        let loads = capture
            .records()
            .into_iter()
            .filter(|line| line.contains("provider catalog loaded") && line.contains(&owner))
            .count();
        assert_eq!(
            loads, 1,
            "six clients must not launch six native catalog loads"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    // MP-08 / MP-10 / MP-11: a delayed native-source scope cannot hold another account's static catalog.
    #[tokio::test]
    async fn amfix_r19_catalog_unrelated_owner_progresses_during_slow_load() {
        crate::test_support::isolated_env_test!();
        let root = std::env::temp_dir().join(format!(
            "amfix-catalog-independent-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let mut slow_config = crate::config::DaemonConfig::for_tests();
        slow_config.provider_catalog_read_delay_ms = 1_000;
        let slow_config = DaemonConfigProjectionStore::new(slow_config);
        let fast_config =
            DaemonConfigProjectionStore::new(crate::config::DaemonConfig::for_tests());
        let projection = ProviderCatalogProjectionStore::default();
        // No linked profiles: exercise the offline catalog, without starting a
        // native model probe whose startup time is unrelated to lock independence.
        let request = crate::local::GetProviderCatalogRequest::default();
        let slow = execute_get_provider_catalog_request(
            &projection,
            &slow_config,
            &registry,
            "owner-a",
            request.clone(),
        );
        tokio::pin!(slow);
        tokio::select! {
            biased;
            result = &mut slow => panic!("slow load must remain pending: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(30)) => {},
        }
        let (other, first) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_millis(250),
                execute_get_provider_catalog_request(
                    &projection,
                    &fast_config,
                    &registry,
                    "owner-b",
                    request
                )
            ),
            slow,
        );
        assert!(first.is_ok(), "{first:?}");
        assert!(
            other.is_ok(),
            "an unrelated cold Claude catalog waited for another owner's slow native load"
        );
        assert!(other.unwrap().is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn amfix_r18_catalog_keeps_replica_and_home_cache_namespaces_distinct() {
        crate::test_support::isolated_env_test!();
        let root =
            std::env::temp_dir().join(format!("chariox-amfix-catalog-{}", rand::random::<u64>()));
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let replica = registry
            .create_managed("cloud-owner", "claude", "Replica")
            .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "cloud-owner".into(),
            ..Default::default()
        });
        let config = DaemonConfigProjectionStore::new(config);
        let projection = ProviderCatalogProjectionStore::default();
        let request = crate::local::GetProviderCatalogRequest {
            provider: Some("claude".into()),
            account_profiles: BTreeMap::from([("claude".into(), replica.profile_id)]),
            ..Default::default()
        };
        // A home cache with the same explicit reference must never answer for the replica.
        projection.update_scoped(
            &provider_catalog_cache_key("local", &request),
            crate::provider::OpenCodeProviderCatalog {
                all: vec![],
                default: BTreeMap::new(),
                connected: vec!["home-cache".into()],
            },
        );
        let response = execute_get_provider_catalog_request(
            &projection,
            &config,
            &registry,
            "cloud-owner",
            request.clone(),
        )
        .await
        .unwrap();
        let LocalDaemonResponse::ProviderCatalog { catalog } = response else {
            panic!("catalog expected")
        };
        assert!(!catalog.connected.contains(&"home-cache".to_string()));
        assert!(projection
            .cached_scoped(&provider_catalog_cache_key("cloud-owner", &request))
            .is_some());
        let foreign = execute_get_provider_catalog_request(
            &projection,
            &config,
            &registry,
            "foreign-owner",
            request,
        )
        .await;
        assert!(
            foreign.is_err(),
            "a foreign caller cannot resolve the replica or its cache"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
