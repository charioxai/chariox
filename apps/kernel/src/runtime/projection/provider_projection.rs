use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use crate::provider::{OpenCodeProviderCatalog, ProviderProcessInfo, RuntimeProviderRun};

use super::ProviderCatalogHealthSnapshot;

#[derive(Clone, Default)]
pub(crate) struct ProviderRunProjectionStore {
    runs: Arc<StdMutex<HashMap<String, RuntimeProviderRun>>>,
}

impl ProviderRunProjectionStore {
    pub(crate) fn get(&self, provider_run_id: &str) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .get(provider_run_id)
            .cloned()
    }

    pub(crate) fn get_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .values()
            .find(|run| run.runtime_mcp_auth_token() == Some(auth_token))
            .cloned()
    }

    pub(crate) fn get_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .max_by_key(|run| match run.state() {
                crate::provider::ProviderRunState::Running => 3,
                crate::provider::ProviderRunState::Parked => 2,
                crate::provider::ProviderRunState::Starting => 1,
                crate::provider::ProviderRunState::Ended => 0,
            })
            .cloned()
    }

    pub(crate) fn list_for_session(&self, session_id: &str) -> Vec<RuntimeProviderRun> {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .values()
            .filter(|run| run.session_id() == session_id)
            .cloned()
            .collect()
    }

    pub(crate) fn update(&self, run: RuntimeProviderRun) {
        self.runs
            .lock()
            .expect("provider run projection lock should not be poisoned")
            .insert(run.id().to_string(), run);
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProviderProcessProjectionStore {
    processes: Arc<StdMutex<Option<Vec<ProviderProcessInfo>>>>,
}

impl ProviderProcessProjectionStore {
    pub(crate) fn list(&self, provider: Option<&str>) -> Option<Vec<ProviderProcessInfo>> {
        let processes = self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned")
            .clone()?;
        Some(filter_provider_processes(processes, provider))
    }

    pub(crate) fn update_list(&self, processes: Vec<ProviderProcessInfo>) {
        *self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned") = Some(processes);
    }

    pub(crate) fn invalidate(&self) {
        *self
            .processes
            .lock()
            .expect("provider process projection lock should not be poisoned") = None;
    }
}

fn filter_provider_processes(
    processes: Vec<ProviderProcessInfo>,
    provider: Option<&str>,
) -> Vec<ProviderProcessInfo> {
    let Some(provider) = provider else {
        return processes;
    };
    processes
        .into_iter()
        .filter(|process| process.provider == provider)
        .collect()
}

#[derive(Clone, Default)]
pub(crate) struct ProviderCatalogProjectionStore {
    catalog: Arc<StdMutex<Option<CachedProviderCatalogProjection>>>,
}

#[derive(Clone)]
struct CachedProviderCatalogProjection {
    cached_at: Instant,
    catalog: OpenCodeProviderCatalog,
}

impl ProviderCatalogProjectionStore {
    pub(crate) fn get(&self, ttl: Duration) -> Option<OpenCodeProviderCatalog> {
        let cached = self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned")
            .clone()?;
        if cached.cached_at.elapsed() < ttl {
            Some(cached.catalog)
        } else {
            None
        }
    }

    pub(crate) fn update(&self, catalog: OpenCodeProviderCatalog) {
        *self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned") =
            Some(CachedProviderCatalogProjection {
                cached_at: Instant::now(),
                catalog,
            });
    }

    pub(crate) fn invalidate(&self) {
        *self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned") = None;
    }

    pub(crate) fn health_snapshot(&self, ttl: Duration) -> ProviderCatalogHealthSnapshot {
        let cached = self
            .catalog
            .lock()
            .expect("provider catalog projection lock should not be poisoned")
            .clone();
        let Some(cached) = cached else {
            return ProviderCatalogHealthSnapshot {
                cached: false,
                expired: false,
                age_ms: None,
                ttl_ms: ttl.as_millis() as u64,
            };
        };
        let age = cached.cached_at.elapsed();
        ProviderCatalogHealthSnapshot {
            cached: true,
            expired: age >= ttl,
            age_ms: Some(age.as_millis() as u64),
            ttl_ms: ttl.as_millis() as u64,
        }
    }
}
