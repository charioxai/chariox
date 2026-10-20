use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use crate::provider::{
    OpenCodeProviderCatalog, ProviderClientInterface, ProviderProcessInfo, ProviderRunState,
    RuntimeProviderRun,
};
use crate::session::RuntimeSession;

use super::{
    ProviderCatalogHealthSnapshot, ProviderRunAgentBindingConflict, ProviderRunHealthSnapshot,
    ProviderRunIdentityIssue, ProviderRunSessionPointerIssue, ProviderRunTerminalDiagnosticIssue,
};

#[cfg(test)]
type LeasedProviderRunProbe = Arc<dyn Fn(&str) + Send + Sync + 'static>;

#[derive(Clone, Default)]
pub(crate) struct ProviderRunProjectionStore {
    runs: Arc<StdMutex<HashMap<String, RuntimeProviderRun>>>,
    leased_provider_run_ids: Arc<StdMutex<HashSet<String>>>,
    catalog_changes: crate::runtime::runtime_tool_catalog::RuntimeToolCatalogChanges,
    #[cfg(test)]
    leased_provider_run_probe: Arc<StdMutex<Option<LeasedProviderRunProbe>>>,
}

impl ProviderRunProjectionStore {
    pub(crate) fn catalog_changes(
        &self,
    ) -> &crate::runtime::runtime_tool_catalog::RuntimeToolCatalogChanges {
        &self.catalog_changes
    }
    pub(crate) fn mark_leased_provider_run(&self, provider_run_id: &str) {
        self.leased_provider_run_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider_run_id.to_string());
    }

    #[cfg(test)]
    pub(crate) fn notify_leased_provider_run_pre_spawn(&self, provider_run_id: &str) {
        #[cfg(test)]
        if let Some(probe) = self
            .leased_provider_run_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            probe(provider_run_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn install_leased_provider_run_probe(
        &self,
        probe: impl Fn(&str) + Send + Sync + 'static,
    ) {
        *self
            .leased_provider_run_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(probe));
    }

    pub(crate) fn is_leased_provider_run(&self, provider_run_id: &str) -> bool {
        self.leased_provider_run_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(provider_run_id)
    }

    pub(crate) fn get(&self, provider_run_id: &str) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(provider_run_id)
            .cloned()
    }

    pub(crate) fn active_runs_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<RuntimeProviderRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|run| {
                run.runtime_mcp_auth_token() == Some(auth_token)
                    && run.state() != ProviderRunState::Ended
            })
            .cloned()
            .collect()
    }

    pub(crate) fn get_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .max_by(|left, right| left.active_selection_cmp(right))
            .cloned()
    }

    pub(crate) fn list_for_session(&self, session_id: &str) -> Vec<RuntimeProviderRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|run| run.session_id() == session_id)
            .cloned()
            .collect()
    }

    pub(crate) fn list(&self) -> Vec<RuntimeProviderRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn update(&self, run: RuntimeProviderRun) {
        let run_id = run.id().to_owned();
        let ended = run.state() == ProviderRunState::Ended;
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = runs.get(&run_id).is_some_and(|previous| {
            previous.remote_extension_manifest() != run.remote_extension_manifest()
        });
        runs.insert(run_id.clone(), run);
        drop(runs);
        if ended {
            self.catalog_changes.close(&run_id);
        } else if changed {
            self.catalog_changes.invalidate(&run_id);
        }
    }

    pub(crate) fn retire_leased_agent_runs(
        &self,
        session_id: &str,
        agent_id: &str,
        leased_agent_id: &str,
    ) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut retired = Vec::new();
        for run in runs.values_mut() {
            if run.session_id() == session_id
                && run.agent_instance_id() == Some(agent_id)
                && run.state() != ProviderRunState::Ended
                && crate::provider::worker_provider_run_id_from_projected_leased_id(
                    leased_agent_id,
                    run.id(),
                )
                .is_some()
            {
                run.mark_ended();
                retired.push(run.id().to_string());
            }
        }
        drop(runs);
        for run_id in retired {
            self.catalog_changes.close(&run_id);
        }
    }

    pub(crate) fn update_remote_snapshot(&self, run: RuntimeProviderRun) -> RuntimeProviderRun {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(current) = runs.get(run.id()) {
            if current.state() == ProviderRunState::Ended
                || (run.state() == ProviderRunState::Starting
                    && current.state() != ProviderRunState::Starting)
            {
                return current.clone();
            }
        }
        // The remote projection admission check has already selected this run.
        // A replacement retires the previous projection for this lease/agent;
        // worker snapshots only describe the newly selected run.
        if run.state() != ProviderRunState::Ended {
            if let (Some((lease_id, _)), Some(agent_id)) = (
                run.id()
                    .strip_prefix("leased:")
                    .and_then(|id| id.split_once(':')),
                run.agent_instance_id(),
            ) {
                let prefix = format!("leased:{lease_id}:");
                for current in runs.values_mut() {
                    if current.id() != run.id()
                        && current.id().starts_with(&prefix)
                        && current.session_id() == run.session_id()
                        && current.agent_instance_id() == Some(agent_id)
                    {
                        current.mark_ended();
                    }
                }
            }
        }
        runs.insert(run.id().to_string(), run.clone());
        run
    }

    pub(crate) fn health_snapshot(
        &self,
        sessions: Vec<RuntimeSession>,
    ) -> ProviderRunHealthSnapshot {
        provider_run_health_snapshot(self.list(), sessions)
    }
}

fn provider_run_health_snapshot(
    runs: Vec<RuntimeProviderRun>,
    sessions: Vec<RuntimeSession>,
) -> ProviderRunHealthSnapshot {
    let session_agents = sessions
        .iter()
        .map(|session| {
            (
                session.id().to_string(),
                session
                    .agents()
                    .iter()
                    .map(|agent| agent.id().to_string())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let runs_by_id = runs
        .iter()
        .map(|run| (run.id().to_string(), run))
        .collect::<BTreeMap<_, _>>();

    let mut active_runs = 0;
    let mut chariox_active_runs = 0;
    let mut native_tui_active_runs = 0;
    let mut active_chariox_bindings: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut active_native_tui_bindings: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut active_agent_bindings: BTreeMap<(String, String), Vec<(String, &'static str)>> =
        BTreeMap::new();
    let mut orphaned_active_runs = Vec::new();
    let mut terminal_diagnostics = Vec::new();

    for run in &runs {
        if run.state() == ProviderRunState::Ended {
            continue;
        }
        active_runs += 1;
        if let Some(diagnostic) = run.terminal_diagnostic() {
            terminal_diagnostics.push(ProviderRunTerminalDiagnosticIssue {
                provider_run_id: run.id().to_string(),
                session_id: run.session_id().to_string(),
                agent_id: run.agent_instance_id().map(str::to_string),
                provider: run.provider().to_string(),
                state: format!("{:?}", run.state()),
                diagnostic: diagnostic.to_string(),
            });
        }
        match run.client_interface() {
            ProviderClientInterface::Chariox => {
                chariox_active_runs += 1;
                if let Some(agent_id) = run.agent_instance_id() {
                    active_chariox_bindings
                        .entry((run.session_id().to_string(), agent_id.to_string()))
                        .or_default()
                        .push(run.id().to_string());
                }
            }
            ProviderClientInterface::NativeTui => {
                native_tui_active_runs += 1;
                if let Some(agent_id) = run.agent_instance_id() {
                    active_native_tui_bindings
                        .entry((run.session_id().to_string(), agent_id.to_string()))
                        .or_default()
                        .push(run.id().to_string());
                }
            }
        }
        if let Some(agent_id) = run.agent_instance_id() {
            active_agent_bindings
                .entry((run.session_id().to_string(), agent_id.to_string()))
                .or_default()
                .push((
                    run.id().to_string(),
                    provider_client_interface_key(run.client_interface()),
                ));
        }
        match session_agents.get(run.session_id()) {
            None => orphaned_active_runs.push(ProviderRunIdentityIssue {
                provider_run_id: run.id().to_string(),
                session_id: run.session_id().to_string(),
                agent_id: run.agent_instance_id().map(str::to_string),
                details: "provider run points at a missing session".to_string(),
            }),
            Some(agents) => {
                if let Some(agent_id) = run.agent_instance_id() {
                    if !agents.contains(agent_id) {
                        orphaned_active_runs.push(ProviderRunIdentityIssue {
                            provider_run_id: run.id().to_string(),
                            session_id: run.session_id().to_string(),
                            agent_id: Some(agent_id.to_string()),
                            details: "provider run points at an agent outside its session"
                                .to_string(),
                        });
                    }
                }
            }
        }
    }

    let duplicate_chariox_agent_bindings = active_chariox_bindings
        .into_iter()
        .filter_map(|((session_id, agent_id), mut provider_run_ids)| {
            provider_run_ids.sort();
            (provider_run_ids.len() > 1).then_some(ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids,
            })
        })
        .collect();

    let duplicate_native_tui_agent_bindings = active_native_tui_bindings
        .into_iter()
        .filter_map(|((session_id, agent_id), mut provider_run_ids)| {
            provider_run_ids.sort();
            (provider_run_ids.len() > 1).then_some(ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids,
            })
        })
        .collect();

    let multi_interface_agent_bindings = active_agent_bindings
        .into_iter()
        .filter_map(|((session_id, agent_id), bindings)| {
            let interfaces = bindings
                .iter()
                .map(|(_, client_interface)| *client_interface)
                .collect::<BTreeSet<_>>();
            if bindings.len() <= 1 || interfaces.len() <= 1 {
                return None;
            }
            let mut provider_run_ids = bindings
                .into_iter()
                .map(|(provider_run_id, client_interface)| {
                    format!("{provider_run_id}:{client_interface}")
                })
                .collect::<Vec<_>>();
            provider_run_ids.sort();
            Some(ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids,
            })
        })
        .collect();

    let session_active_run_mismatches = sessions
        .iter()
        .filter_map(|session| {
            let active_provider_run_id = session.active_provider_run_id()?;
            let Some(run) = runs_by_id.get(active_provider_run_id) else {
                return Some(ProviderRunSessionPointerIssue {
                    session_id: session.id().to_string(),
                    active_provider_run_id: Some(active_provider_run_id.to_string()),
                    details: "active provider run is not projected".to_string(),
                });
            };
            if run.session_id() != session.id() {
                return Some(ProviderRunSessionPointerIssue {
                    session_id: session.id().to_string(),
                    active_provider_run_id: Some(active_provider_run_id.to_string()),
                    details: format!("active provider run points at session {}", run.session_id()),
                });
            }
            if run.state() == ProviderRunState::Ended {
                return Some(ProviderRunSessionPointerIssue {
                    session_id: session.id().to_string(),
                    active_provider_run_id: Some(active_provider_run_id.to_string()),
                    details: "active provider run is ended".to_string(),
                });
            }
            if let (Some(focused_agent_id), Some(run_agent_id)) =
                (session.focused_agent_id(), run.agent_instance_id())
            {
                if focused_agent_id != run_agent_id {
                    let focused_agent_has_live_run = runs.iter().any(|candidate| {
                        candidate.session_id() == session.id()
                            && candidate.state() != ProviderRunState::Ended
                            && candidate.agent_instance_id() == Some(focused_agent_id)
                    });
                    if !focused_agent_has_live_run {
                        return Some(ProviderRunSessionPointerIssue {
                            session_id: session.id().to_string(),
                            active_provider_run_id: Some(active_provider_run_id.to_string()),
                            details: format!(
                                "active provider run points at agent {run_agent_id}, focused agent is {focused_agent_id}"
                            ),
                        });
                    }
                }
            }
            None
        })
        .collect();

    ProviderRunHealthSnapshot {
        projected_runs: runs.len(),
        active_runs,
        chariox_active_runs,
        native_tui_active_runs,
        terminal_diagnostics,
        duplicate_chariox_agent_bindings,
        duplicate_native_tui_agent_bindings,
        multi_interface_agent_bindings,
        orphaned_active_runs,
        session_active_run_mismatches,
    }
}

fn provider_client_interface_key(client_interface: ProviderClientInterface) -> &'static str {
    match client_interface {
        ProviderClientInterface::Chariox => "chariox",
        ProviderClientInterface::NativeTui => "native_tui",
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        Some(filter_provider_processes(processes, provider))
    }

    pub(crate) fn update_list(&self, processes: Vec<ProviderProcessInfo>) {
        *self
            .processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(processes);
    }

    pub(crate) fn invalidate(&self) {
        *self
            .processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
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
    catalogs: Arc<StdMutex<BTreeMap<String, CachedProviderCatalogProjection>>>,
    changes: Arc<tokio::sync::Notify>,
    revision: Arc<std::sync::atomic::AtomicU64>,
    refresh_in_progress: Arc<StdMutex<BTreeSet<String>>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone)]
struct CachedProviderCatalogProjection {
    cached_at: Instant,
    generation: u64,
    catalog: OpenCodeProviderCatalog,
}

impl ProviderCatalogProjectionStore {
    pub(crate) fn get(&self, ttl: Duration) -> Option<OpenCodeProviderCatalog> {
        self.get_scoped("", ttl)
    }

    pub(crate) fn get_scoped(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<OpenCodeProviderCatalog> {
        let cached = self
            .catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(cache_key)
            .cloned()?;
        if cached.generation == self.invalidation_generation() && cached.cached_at.elapsed() < ttl {
            Some(cached.catalog)
        } else {
            None
        }
    }

    pub(crate) fn update(&self, catalog: OpenCodeProviderCatalog) {
        self.update_scoped("", catalog);
    }

    pub(crate) fn invalidation_generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn change_sequence(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    fn changed(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.changes.notify_waiters();
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        loop {
            let notified = self.changes.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.change_sequence() != sequence {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn update_scoped(&self, cache_key: &str, catalog: OpenCodeProviderCatalog) {
        let changed = {
            let mut catalogs = self
                .catalogs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = catalogs
                .get(cache_key)
                .is_none_or(|previous| previous.catalog != catalog);
            catalogs.insert(
                cache_key.to_string(),
                CachedProviderCatalogProjection {
                    cached_at: Instant::now(),
                    generation: self.invalidation_generation(),
                    catalog,
                },
            );
            changed
        };
        if changed {
            self.changed();
        }
    }

    pub(crate) fn cached(&self) -> Option<OpenCodeProviderCatalog> {
        self.cached_scoped("")
    }

    pub(crate) fn cached_scoped(&self, cache_key: &str) -> Option<OpenCodeProviderCatalog> {
        self.catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(cache_key)
            .map(|cached| cached.catalog.clone())
    }

    pub(crate) fn begin_refresh(&self, cache_key: &str) -> Option<u64> {
        self.refresh_in_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(cache_key.to_string())
            .then(|| self.invalidation_generation())
    }

    pub(crate) fn update_scoped_if_generation(
        &self,
        cache_key: &str,
        catalog: OpenCodeProviderCatalog,
        generation: u64,
    ) -> bool {
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            return false;
        }
        let mut cached = self
            .catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            return false;
        }
        let changed = cached
            .get(cache_key)
            .is_none_or(|previous| previous.catalog != catalog);
        cached.insert(
            cache_key.to_string(),
            CachedProviderCatalogProjection {
                cached_at: Instant::now(),
                generation,
                catalog,
            },
        );
        drop(cached);
        if changed {
            self.changed();
        }
        true
    }

    /// A discovery that an invalidation overtook is still the best known model
    /// metadata for a scope that has none: keep it as stale, never fresh.
    pub(crate) fn update_scoped_stale_if_absent(
        &self,
        cache_key: &str,
        catalog: OpenCodeProviderCatalog,
        generation: u64,
    ) {
        let mut cached = self
            .catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cached.contains_key(cache_key) {
            return;
        }
        cached.insert(
            cache_key.to_string(),
            CachedProviderCatalogProjection {
                cached_at: Instant::now(),
                generation,
                catalog,
            },
        );
        drop(cached);
        self.changed();
    }

    pub(crate) fn finish_refresh(&self, cache_key: &str, generation: u64) {
        self.refresh_in_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(cache_key);
        // A change during discovery needs one fresh attempt after this gate
        // opens. Ordinary completion/failure never creates a polling loop.
        if generation != self.invalidation_generation() {
            self.changed();
        }
    }

    pub(crate) fn invalidate(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        // MP-08 / MP-10 / MP-11: account metadata is read fresh separately.
        // Keep scoped model metadata for the existing stale-while-refresh path,
        // so account changes do not block on provider model discovery. The
        // generation fence still rejects fresh reads and older refresh jobs.
        self.changed();
    }

    pub(crate) fn health_snapshot(&self, ttl: Duration) -> ProviderCatalogHealthSnapshot {
        let cached = self
            .catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get("")
            .cloned();
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
            expired: cached.generation != self.invalidation_generation() || age >= ttl,
            age_ms: Some(age.as_millis() as u64),
            ttl_ms: ttl.as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod provider_catalog_projection_tests {
    use super::*;

    fn catalog(provider: &str) -> OpenCodeProviderCatalog {
        OpenCodeProviderCatalog {
            all: vec![crate::provider::OpenCodeProviderInfo {
                id: provider.to_string(),
                name: provider.to_string(),
                remote_machine_aliases: Vec::new(),
                models: Default::default(),
            }],
            default: Default::default(),
            connected: vec![provider.to_string()],
        }
    }

    #[test]
    fn expired_catalog_remains_available_as_last_known_projection() {
        let store = ProviderCatalogProjectionStore::default();
        store.update(catalog("codex"));

        assert!(store.get(Duration::ZERO).is_none());
        assert_eq!(
            store
                .cached()
                .expect("cached catalog should remain")
                .connected,
            vec!["codex"]
        );
    }

    #[test]
    fn mp08_mp10_invalidation_keeps_models_for_immediate_account_revision_reads() {
        let store = ProviderCatalogProjectionStore::default();
        store.update_scoped("owner-a:claude:work:local", catalog("claude"));
        let sequence = store.change_sequence();
        store.invalidate();
        assert!(store.change_sequence() > sequence);
        assert!(
            store
                .get_scoped("owner-a:claude:work:local", Duration::from_secs(3600))
                .is_none(),
            "invalidated models are never fresh"
        );
        assert_eq!(
            store
                .cached_scoped("owner-a:claude:work:local")
                .expect("stale model metadata must not block fresh account labels")
                .connected,
            vec!["claude"]
        );
        assert!(
            store.cached_scoped("owner-b:claude:work:local").is_none(),
            "stale data stays in its owner/profile/location scope"
        );
    }

    #[test]
    fn explicit_invalidation_rejects_an_inflight_refresh() {
        let store = ProviderCatalogProjectionStore::default();
        store.update(catalog("codex"));
        let generation = store
            .begin_refresh("")
            .expect("first refresh should acquire the gate");

        store.invalidate();

        assert!(!store.update_scoped_if_generation("", catalog("opencode"), generation));
        assert!(store.get(Duration::from_secs(3600)).is_none());
        assert_eq!(
            store
                .cached()
                .expect("only the previous scoped metadata remains")
                .connected,
            vec!["codex"]
        );
        store.finish_refresh("", generation);
        assert!(store.begin_refresh("").is_some());
    }

    #[test]
    fn mp08_mp10_model_refreshes_are_scoped_and_retry_changes_during_discovery() {
        let store = ProviderCatalogProjectionStore::default();
        let generation = store.begin_refresh("owner-a:claude:work:local").unwrap();
        assert!(store.begin_refresh("owner-a:claude:work:local").is_none());
        let other = store
            .begin_refresh("owner-a:claude:personal:local")
            .expect("one scope must not suppress another scope's refresh");
        let sequence = store.change_sequence();
        store.finish_refresh("owner-a:claude:personal:local", other);
        assert_eq!(
            store.change_sequence(),
            sequence,
            "ordinary completion does not poll"
        );
        store.invalidate();
        let sequence = store.change_sequence();
        store.finish_refresh("owner-a:claude:work:local", generation);
        assert!(
            store.change_sequence() > sequence,
            "a newer invalidation is retried after the old job releases its scope"
        );
        assert_eq!(
            store.begin_refresh("owner-a:claude:work:local"),
            Some(store.invalidation_generation())
        );
    }

    #[test]
    fn catalogs_are_isolated_by_owner_profile_and_execution_location_key() {
        let store = ProviderCatalogProjectionStore::default();
        store.update_scoped("owner-a:codex:work:local", catalog("codex-work"));
        store.update_scoped("owner-a:codex:personal:slice-a", catalog("codex-personal"));
        store.update_scoped("owner-b:codex:work:local", catalog("codex-owner-b"));

        assert_eq!(
            store
                .cached_scoped("owner-a:codex:work:local")
                .expect("work catalog")
                .connected,
            vec!["codex-work"]
        );
        assert_eq!(
            store
                .cached_scoped("owner-a:codex:personal:slice-a")
                .expect("personal slice catalog")
                .connected,
            vec!["codex-personal"]
        );
        assert_eq!(
            store
                .cached_scoped("owner-b:codex:work:local")
                .expect("other owner catalog")
                .connected,
            vec!["codex-owner-b"]
        );
    }
}

#[cfg(test)]
mod client_catalog_change_tests {
    use super::*;
    #[tokio::test]
    async fn mp08_mp10_catalog_changes_wake_without_polling_and_ignore_identical_refreshes() {
        let store = ProviderCatalogProjectionStore::default();
        let sequence = store.change_sequence();
        let changed = store.wait_for_change_after(sequence);
        store.invalidate();
        tokio::time::timeout(Duration::from_millis(50), changed)
            .await
            .expect("invalidation wakes even without a warmed catalog");
        let catalog = OpenCodeProviderCatalog {
            all: vec![],
            default: Default::default(),
            connected: vec![],
        };
        store.update_scoped("owner", catalog.clone());
        let sequence = store.change_sequence();
        store.update_scoped("owner", catalog);
        assert_eq!(
            store.change_sequence(),
            sequence,
            "an unchanged refresh must not create an event/fetch loop"
        );
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            store.wait_for_change_after(sequence)
        )
        .await
        .is_err());
    }
}
