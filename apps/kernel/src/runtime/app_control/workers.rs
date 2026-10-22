//! Bounded discovery of actual activated process owners. These weak projections
//! never keep a process alive or grant an agent permission to invoke its tools.
use super::AppControlService;
use crate::runtime::app_worker::{ActivatedApp, AppWorkerError, AppWorkerLease};
use chariox_app_runtime::app_outbox::EventCatalog;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

const MAX_PROJECTIONS: usize = 64;
/// Bound on remembered (agent, App) listing omissions.
const MAX_UNLISTED: usize = 4096;
type Key = (String, String);
struct Dormant {
    catalog: Arc<EventCatalog>,
    configuration: serde_json::Value,
    suspended: bool,
}
/// Live worker projections, plus the verified catalogs of Apps stopped while
/// idle. A dormant catalog keeps tools discoverable; invoking one starts the
/// App on demand. Current generation and signer are rechecked before use.
/// The third part remembers which agents' tool listings left a bound App
/// out; the fourth signals when such an App starts.
#[derive(Clone, Default)]
pub(super) struct ActiveWorkers(
    Arc<Mutex<BTreeMap<Key, ActivatedApp>>>,
    Arc<Mutex<BTreeMap<Key, Dormant>>>,
    Arc<Mutex<Unlisted>>,
    Arc<tokio::sync::Notify>,
);

/// Agents whose App tool listing left out a bound App (it neither ran nor
/// could start then). When that App starts, they are due a catalog refresh,
/// once per omission.
#[derive(Default)]
struct Unlisted {
    by_app: BTreeMap<Key, BTreeSet<String>>,
    len: usize,
    due: BTreeSet<String>,
}

impl ActiveWorkers {
    pub(super) fn note_unlisted(&self, owner: &str, installation: &str, agent: &str) {
        let Ok(mut unlisted) = self.2.lock() else {
            return;
        };
        let key = (owner.to_owned(), installation.to_owned());
        if unlisted
            .by_app
            .get(&key)
            .is_some_and(|agents| agents.contains(agent))
        {
            return;
        }
        if unlisted.len >= MAX_UNLISTED {
            tracing::debug!(
                installation,
                agent,
                "listing omissions at their bound; this App's start will not refresh the agent"
            );
            return;
        }
        unlisted
            .by_app
            .entry(key)
            .or_default()
            .insert(agent.to_owned());
        unlisted.len += 1;
    }
    pub(super) fn forget_unlisted(&self, owner: &str, installation: &str, agent: &str) {
        let Ok(mut unlisted) = self.2.lock() else {
            return;
        };
        let key = (owner.to_owned(), installation.to_owned());
        let Some(agents) = unlisted.by_app.get_mut(&key) else {
            return;
        };
        if agents.remove(agent) {
            if agents.is_empty() {
                unlisted.by_app.remove(&key);
            }
            unlisted.len -= 1;
        }
    }
    /// The App runs now: agents whose listing left it out are due a refresh.
    fn started(&self, key: &Key) {
        let Ok(mut unlisted) = self.2.lock() else {
            return;
        };
        let Some(agents) = unlisted.by_app.remove(key) else {
            return;
        };
        unlisted.len -= agents.len();
        unlisted.due.extend(agents);
        drop(unlisted);
        self.3.notify_one();
    }
    #[cfg(test)]
    pub(super) fn is_unlisted(&self, owner: &str, installation: &str, agent: &str) -> bool {
        self.2
            .lock()
            .unwrap()
            .by_app
            .get(&(owner.to_owned(), installation.to_owned()))
            .is_some_and(|agents| agents.contains(agent))
    }
    pub(super) fn take_due(&self) -> Vec<String> {
        self.2.lock().map_or_else(
            |_| Vec::new(),
            |mut unlisted| std::mem::take(&mut unlisted.due).into_iter().collect(),
        )
    }
    pub(super) fn due_signal(&self) -> Arc<tokio::sync::Notify> {
        self.3.clone()
    }
    fn is_dormant(&self, owner: &str, installation: &str) -> bool {
        self.1
            .lock()
            .is_ok_and(|dormant| dormant.contains_key(&(owner.to_owned(), installation.to_owned())))
    }
    fn forget_dormant(&self, owner: &str, installation: &str) {
        if let Ok(mut dormant) = self.1.lock() {
            dormant.remove(&(owner.to_owned(), installation.to_owned()));
        }
    }
}

/// Retained by lifecycle threads without retaining AppControl/the lifecycle
/// service itself. This avoids a service->thread->service shutdown cycle.
#[derive(Clone)]
pub(crate) struct AppWorkerPublisher {
    workers: ActiveWorkers,
    event_pump: crate::runtime::app_event_pump::AppEventPump,
}
impl AppWorkerPublisher {
    pub(super) fn new(
        workers: ActiveWorkers,
        event_pump: crate::runtime::app_event_pump::AppEventPump,
    ) -> Self {
        Self {
            workers,
            event_pump,
        }
    }
    pub(crate) fn publish(&self, owner: &str, worker: ActivatedApp) -> Result<(), AppWorkerError> {
        let lease = worker.lease(owner)?;
        let key = (
            lease.owner().to_owned(),
            lease.catalog().installation_id().to_owned(),
        );
        let mut workers = self
            .workers
            .0
            .lock()
            .map_err(|_| AppWorkerError::Unavailable)?;
        workers.retain(|(owner, _), worker| worker.lease(owner).is_ok());
        if workers.contains_key(&key) || workers.len() >= MAX_PROJECTIONS {
            return Err(AppWorkerError::Busy);
        }
        workers.insert(key.clone(), worker);
        drop(workers);
        if let Ok(mut dormant) = self.workers.1.lock() {
            dormant.remove(&key);
        }
        self.workers.started(&key);
        self.event_pump.wake();
        Ok(())
    }
    pub(crate) fn is_dormant(&self, owner: &str, installation: &str) -> bool {
        self.workers.is_dormant(owner, installation)
    }
    /// Seed a verified release for first-call discovery without claiming it
    /// previously ran or sending a resume callback on its first start.
    pub(crate) fn retain_dormant(&self, owner: &str, catalog: Arc<EventCatalog>) -> bool {
        let Ok(mut dormant) = self.workers.1.lock() else {
            return false;
        };
        let key = (owner.to_owned(), catalog.installation_id().to_owned());
        if dormant.contains_key(&key) {
            return true;
        }
        if dormant.len() >= MAX_PROJECTIONS {
            return false;
        }
        dormant.insert(
            key,
            Dormant {
                catalog,
                configuration: serde_json::Value::Null,
                suspended: false,
            },
        );
        true
    }
    pub(crate) fn is_suspended(&self, owner: &str, installation: &str) -> bool {
        self.workers.1.lock().is_ok_and(|dormant| {
            dormant
                .get(&(owner.to_owned(), installation.to_owned()))
                .is_some_and(|entry| entry.suspended)
        })
    }
    /// Last notified configuration, retained with the dormant catalog.
    pub(crate) fn dormant_configuration(
        &self,
        owner: &str,
        installation: &str,
    ) -> Option<serde_json::Value> {
        self.workers
            .1
            .lock()
            .ok()?
            .get(&(owner.into(), installation.into()))
            .filter(|d| d.suspended)
            .map(|d| d.configuration.clone())
    }
    /// Reserve bounded capacity before draining or invoking App code. Reserved
    /// catalogs remain discoverable while suspension is pending. Configuration
    /// and wake-pump keys remain commit-only until suspension succeeds.
    pub(crate) fn reserve_dormant(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        configuration: serde_json::Value,
    ) -> Option<DormantReservation> {
        let mut dormant = self.workers.1.lock().ok()?;
        let key = (owner.to_owned(), catalog.installation_id().to_owned());
        if dormant.contains_key(&key) || dormant.len() >= MAX_PROJECTIONS {
            return None;
        }
        dormant.insert(
            key.clone(),
            Dormant {
                catalog,
                configuration,
                suspended: false,
            },
        );
        Some(DormantReservation {
            workers: self.workers.clone(),
            key,
            committed: false,
        })
    }
    pub(crate) fn forget_dormant(&self, owner: &str, installation: &str) {
        self.workers.forget_dormant(owner, installation);
    }
}

/// The owner retains only a capacity reservation, never a registry lock.
pub(crate) struct DormantReservation {
    workers: ActiveWorkers,
    key: Key,
    committed: bool,
}
impl DormantReservation {
    pub(crate) fn commit(mut self) -> bool {
        let mut dormant = self
            .workers
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = dormant.get_mut(&self.key) else {
            return false;
        };
        entry.suspended = true;
        self.committed = true;
        true
    }
}
impl Drop for DormantReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.workers.forget_dormant(&self.key.0, &self.key.1);
        }
    }
}

impl AppControlService {
    /// The blocking lifecycle owner publishes only after the exact process's
    /// readiness and durable activation proof. It must retain AppWorkerOwner and
    /// serialize replacement for this installation through its complete drain.
    /// This registry provides discovery, not lifecycle admission or resource caps.
    #[cfg(test)]
    pub(crate) fn publish_app_worker(
        &self,
        owner: &str,
        worker: ActivatedApp,
    ) -> Result<(), AppWorkerError> {
        AppWorkerPublisher::new(self.workers.clone(), self.event_pump.clone())
            .publish(owner, worker)
    }

    pub(crate) fn active_app_lease(
        &self,
        owner: &str,
        installation: &str,
    ) -> Option<AppWorkerLease> {
        self.workers
            .0
            .lock()
            .ok()?
            .get(&(owner.to_owned(), installation.to_owned()))?
            .lease(owner)
            .ok()
    }

    /// A refused or failed on-demand start withdraws the dormant catalog, so
    /// agents stop seeing tools that cannot start.
    pub(crate) fn forget_app_dormant(&self, owner: &str, installation: &str) {
        self.workers.forget_dormant(owner, installation);
    }

    pub(crate) fn is_app_dormant(&self, owner: &str, installation: &str) -> bool {
        self.workers.is_dormant(owner, installation)
    }

    pub(crate) fn dormant_app_keys(&self) -> Vec<(String, String)> {
        self.workers.1.lock().map_or_else(
            |_| Vec::new(),
            |dormant| {
                dormant
                    .iter()
                    .filter(|(_, d)| d.suspended)
                    .map(|(key, _)| key.clone())
                    .collect()
            },
        )
    }

    /// Dormant (idle-stopped) catalogs for this owner, at most 64.
    pub(crate) fn dormant_app_catalogs(&self, owner: &str) -> Vec<Arc<EventCatalog>> {
        self.workers.1.lock().map_or_else(
            |_| Vec::new(),
            |dormant| {
                dormant
                    .iter()
                    .filter(|((key_owner, _), _)| key_owner == owner)
                    .map(|(_, dormant)| dormant.catalog.clone())
                    .collect()
            },
        )
    }

    /// Strict lexicographic cursor; the pump wraps explicitly after a short page.
    /// Bounds apply to returned leases and to the backing registry independently.
    pub(crate) fn active_app_leases(
        &self,
        after: Option<(&str, &str)>,
        limit: usize,
    ) -> Vec<AppWorkerLease> {
        let Ok(workers) = self.workers.0.lock() else {
            return Vec::new();
        };
        workers
            .iter()
            .filter(|((owner, installation), _)| {
                after.is_none_or(|after| (owner.as_str(), installation.as_str()) > after)
            })
            .filter_map(|((owner, _), worker)| worker.lease(owner).ok())
            .take(limit.min(16))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_makes_agents_whose_listing_left_the_app_out_due_once() {
        let workers = ActiveWorkers::default();
        let key = |app: &str| ("alice".to_owned(), app.to_owned());
        workers.note_unlisted("alice", "todo", "agent-1");
        workers.note_unlisted("alice", "todo", "agent-1");
        workers.note_unlisted("alice", "docs", "agent-2");
        workers.note_unlisted("alice", "docs", "agent-3");
        // Listed (or revoked) since: no refresh when the App starts.
        workers.forget_unlisted("alice", "docs", "agent-3");
        workers.started(&key("todo"));
        assert_eq!(workers.take_due(), vec!["agent-1".to_owned()]);
        assert!(workers.take_due().is_empty());
        workers.started(&key("todo"));
        assert!(workers.take_due().is_empty());
        workers.started(&key("docs"));
        assert_eq!(workers.take_due(), vec!["agent-2".to_owned()]);
        assert_eq!(workers.2.lock().unwrap().len, 0);
    }
}
