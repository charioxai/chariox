//! Keeps clients' view of provider accounts and models current without a client asking.
//!
//! The kernel looks for the provider logins already on this machine when it
//! starts and when a client asks again. Whenever who is signed in changes, from
//! any source, it invalidates the catalog, wakes every client and warms the
//! model catalog so the first picker already has models.

use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::CommandRouter;

/// A client opening the waiting room asks again; asking twice in a minute is one pass.
const MIN_DISCOVERY_INTERVAL: Duration = Duration::from_secs(60);

/// `watch::Ref` is not `Send`, so the wait keeps only the fact that shutdown began.
async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stopped| *stopped).await.map(|_| ());
}

impl CommandRouter {
    pub(crate) async fn run_provider_discovery(&self, mut shutdown: watch::Receiver<bool>) {
        let registry = self
            .runtime_state
            .provider_account_profile_registry()
            .clone();
        let mut seen_auth_revision = registry.auth_revision();
        let mut last_pass = Instant::now();
        tokio::select! {
            _ = stopped(&mut shutdown) => return,
            () = self.discover_machine_providers() => {}
        }
        loop {
            tokio::select! {
                _ = stopped(&mut shutdown) => return,
                () = registry.wait_for_auth_change_after(seen_auth_revision) => {
                    seen_auth_revision = registry.auth_revision();
                    self.publish_account_change().await;
                }
                () = registry.wait_for_discovery_request() => {
                    if last_pass.elapsed() >= MIN_DISCOVERY_INTERVAL {
                        last_pass = Instant::now();
                        self.discover_machine_providers().await;
                    }
                }
            }
        }
    }

    /// One discovery pass, then a warm catalog even when nothing changed.
    pub(crate) async fn discover_machine_providers(&self) {
        let Some(home) = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
        else {
            return;
        };
        let registry = self
            .runtime_state
            .provider_account_profile_registry()
            .clone();
        let owner = self
            .runtime_state
            .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
        let config = self.config_projection.snapshot();
        let outcome = tokio::task::spawn_blocking(move || {
            crate::provider_discovery::discover_machine_providers(&registry, &owner, &home, &config)
        })
        .await;
        match outcome {
            Ok(outcome) => crate::logging::info_with_fields(
                "daemon.provider_discovery",
                "machine provider discovery finished",
                serde_json::json!({
                    "validated": outcome.validated.len(),
                    "adopted": outcome.adopted.len(),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.provider_discovery",
                "machine provider discovery task failed",
                serde_json::json!({ "error": error.to_string() }),
            ),
        }
        self.warm_provider_catalog().await;
    }

    async fn publish_account_change(&self) {
        self.runtime_state
            .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
            .await;
        self.runtime_state.record_waiting_room_change();
        self.warm_provider_catalog().await;
    }

    async fn warm_provider_catalog(&self) {
        let owner = self
            .runtime_state
            .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
        let read = super::super::provider_catalog_control::read_catalog(
            &self.provider_catalog_projection,
            &self.config_projection,
            self.runtime_state.provider_account_profile_registry(),
            &owner,
            crate::local::GetProviderCatalogRequest::default(),
            true,
            false,
        )
        .await;
        if let Err(error) = read {
            crate::logging::info_with_fields(
                "daemon.provider_discovery",
                "provider catalog could not be warmed",
                serde_json::json!({ "error": error.to_string() }),
            );
        }
    }
}

#[cfg(test)]
impl CommandRouter {
    /// What a client asking for the default catalog would be answered from memory.
    pub(crate) fn warmed_default_catalog(
        &self,
    ) -> Option<crate::provider::OpenCodeProviderCatalog> {
        let owner = self
            .runtime_state
            .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
        let key = super::super::provider_catalog_control::provider_catalog_cache_key(
            &owner,
            &crate::local::GetProviderCatalogRequest::default(),
        );
        self.provider_catalog_projection.cached_scoped(&key)
    }
}

#[cfg(test)]
#[path = "provider_discovery_tests.rs"]
mod tests;
