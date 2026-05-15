use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::provider::{AgentEndpointMode, OpenCodeProviderCatalog};

#[derive(Debug, Clone)]
pub(crate) struct TrackedProviderProcess {
    pub(crate) process_id: String,
    pub(crate) pid: Option<u32>,
    pub(crate) endpoint_mode: AgentEndpointMode,
    pub(crate) process_label: String,
    pub(crate) started_at_ms: u64,
    pub(crate) owner_provider_run_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderProcessTrackingStore {
    inner: Arc<Mutex<ProviderProcessTrackingState>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderProcessTrackingState {
    pub(crate) processes: BTreeMap<String, TrackedProviderProcess>,
    pub(crate) run_processes: BTreeMap<String, String>,
}

impl ProviderProcessTrackingStore {
    pub(crate) fn read(&self) -> MutexGuard<'_, ProviderProcessTrackingState> {
        self.inner
            .lock()
            .expect("provider process tracking mutex poisoned")
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, ProviderProcessTrackingState> {
        self.inner
            .lock()
            .expect("provider process tracking mutex poisoned")
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> ProviderProcessTrackingState {
        self.read().clone()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderCatalogCacheStore {
    inner: Arc<Mutex<Option<(Instant, OpenCodeProviderCatalog)>>>,
}

impl ProviderCatalogCacheStore {
    pub(crate) fn get_fresh(&self, ttl: Duration) -> Option<OpenCodeProviderCatalog> {
        let cache = self
            .inner
            .lock()
            .expect("provider catalog cache mutex poisoned");
        let Some((cached_at, catalog)) = &*cache else {
            return None;
        };
        (cached_at.elapsed() < ttl).then(|| catalog.clone())
    }

    pub(crate) fn set(&self, catalog: OpenCodeProviderCatalog) {
        *self
            .inner
            .lock()
            .expect("provider catalog cache mutex poisoned") = Some((Instant::now(), catalog));
    }

    pub(crate) fn clear(&self) {
        *self
            .inner
            .lock()
            .expect("provider catalog cache mutex poisoned") = None;
    }
}
