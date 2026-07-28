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
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderCatalog(_) => {
            execute_get_provider_catalog_request(provider_catalog_projection, config_projection)
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
) -> Result<LocalDaemonResponse, DaemonError> {
    if let Some(catalog) = provider_catalog_projection.get(PROVIDER_CATALOG_CACHE_TTL) {
        crate::logging::info_with_fields(
            "daemon.startup",
            "provider catalog cache hit",
            serde_json::json!({
                "provider_count": catalog.all.len(),
                "connected_provider_count": catalog.connected.len(),
            }),
        );
        return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
    }
    if let Some(catalog) = provider_catalog_projection.cached() {
        refresh_provider_catalog_in_background(
            provider_catalog_projection.clone(),
            config_projection.snapshot(),
        );
        crate::logging::info_with_fields(
            "daemon.startup",
            "serving stale provider catalog while refresh runs",
            serde_json::json!({
                "provider_count": catalog.all.len(),
                "connected_provider_count": catalog.connected.len(),
            }),
        );
        return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
    }

    let config = config_projection.snapshot();
    let load_started = Instant::now();
    let catalog = tokio::task::spawn_blocking(move || load_provider_catalog(config))
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
    provider_catalog_projection.update(catalog.clone());
    Ok(LocalDaemonResponse::ProviderCatalog { catalog })
}

fn refresh_provider_catalog_in_background(
    provider_catalog_projection: ProviderCatalogProjectionStore,
    config: crate::config::DaemonConfig,
) {
    let Some(generation) = provider_catalog_projection.begin_refresh() else {
        return;
    };
    tokio::spawn(async move {
        let load_started = Instant::now();
        let result = tokio::task::spawn_blocking(move || load_provider_catalog(config)).await;
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
                if !provider_catalog_projection.update_if_generation(catalog, generation) {
                    crate::logging::info(
                        "daemon.startup",
                        "discarded provider catalog refresh after explicit invalidation",
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
        provider_catalog_projection.finish_refresh();
    });
}

pub(crate) fn execute_get_provider_command_catalogs_request(
) -> Result<LocalDaemonResponse, DaemonError> {
    provider_command_catalogs_response()
}

pub(crate) async fn provider_catalog_json_value(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
) -> Option<serde_json::Value> {
    if let Some(catalog) = provider_catalog_projection.get(PROVIDER_CATALOG_CACHE_TTL) {
        return serde_json::to_value(catalog).ok();
    }
    let config = config_projection.snapshot();
    let load_started = Instant::now();
    tokio::task::spawn_blocking(move || load_provider_catalog(config))
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
