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
    let owner_user_id = crate::account_profile::provider_account_authority_owner_user_id(
        &config_projection.snapshot(),
        owner_user_id,
    );
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

    let generation = provider_catalog_projection.invalidation_generation();
    let config = config_projection.snapshot();
    let registry = account_profiles.clone();
    let load_started = Instant::now();
    let catalog = tokio::task::spawn_blocking(move || {
        load_provider_catalog(config, registry, owner_user_id, request)
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
