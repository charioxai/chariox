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

#[derive(Clone, Default)]
pub(crate) struct ProviderRunProjectionStore {
    runs: Arc<StdMutex<HashMap<String, RuntimeProviderRun>>>,
    leased_provider_run_ids: Arc<StdMutex<HashSet<String>>>,
}

impl ProviderRunProjectionStore {
    pub(crate) fn mark_leased_provider_run(&self, provider_run_id: &str) {
        self.leased_provider_run_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider_run_id.to_string());
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
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run.id().to_string(), run);
    }

    pub(crate) fn update_remote_snapshot(&self, run: RuntimeProviderRun) -> RuntimeProviderRun {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(current) = runs.get(run.id()) {
            if run.state() == ProviderRunState::Starting
                && current.state() != ProviderRunState::Starting
            {
                return current.clone();
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
                    return Some(ProviderRunSessionPointerIssue {
                        session_id: session.id().to_string(),
                        active_provider_run_id: Some(active_provider_run_id.to_string()),
                        details: format!(
                            "active provider run points at agent {run_agent_id}, focused agent is {focused_agent_id}"
                        ),
                    });
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
    catalog: Arc<StdMutex<Option<CachedProviderCatalogProjection>>>,
    refresh_in_progress: Arc<std::sync::atomic::AtomicBool>,
    generation: Arc<std::sync::atomic::AtomicU64>,
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(CachedProviderCatalogProjection {
                cached_at: Instant::now(),
                catalog,
            });
    }

    pub(crate) fn cached(&self) -> Option<OpenCodeProviderCatalog> {
        self.catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|cached| cached.catalog.clone())
    }

    pub(crate) fn begin_refresh(&self) -> Option<u64> {
        self.refresh_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
            .then(|| self.generation.load(std::sync::atomic::Ordering::Acquire))
    }

    pub(crate) fn update_if_generation(
        &self,
        catalog: OpenCodeProviderCatalog,
        generation: u64,
    ) -> bool {
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            return false;
        }
        let mut cached = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.generation.load(std::sync::atomic::Ordering::Acquire) != generation {
            return false;
        }
        *cached = Some(CachedProviderCatalogProjection {
            cached_at: Instant::now(),
            catalog,
        });
        true
    }

    pub(crate) fn finish_refresh(&self) {
        self.refresh_in_progress
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn invalidate(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub(crate) fn health_snapshot(&self, ttl: Duration) -> ProviderCatalogHealthSnapshot {
        let cached = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    fn explicit_invalidation_rejects_an_inflight_refresh() {
        let store = ProviderCatalogProjectionStore::default();
        store.update(catalog("codex"));
        let generation = store
            .begin_refresh()
            .expect("first refresh should acquire the gate");

        store.invalidate();

        assert!(!store.update_if_generation(catalog("opencode"), generation));
        assert!(store.cached().is_none());
        store.finish_refresh();
        assert!(store.begin_refresh().is_some());
    }
}
