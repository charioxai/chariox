//! Bounded discovery of actual activated process owners. These weak projections
//! never keep a process alive or grant an agent permission to invoke its tools.
use super::AppControlService;
use crate::runtime::app_worker::{ActivatedApp, AppWorkerError, AppWorkerLease};
use chariox_app_runtime::app_outbox::EventCatalog;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const MAX_PROJECTIONS: usize = 64;
type Key = (String, String);
/// Live worker projections, plus the verified catalogs of Apps stopped while
/// idle. A dormant catalog keeps tools discoverable; invoking one starts the
/// App on demand. Current generation and signer are rechecked before use.
#[derive(Clone, Default)]
pub(super) struct ActiveWorkers(
    Arc<Mutex<BTreeMap<Key, ActivatedApp>>>,
    Arc<Mutex<BTreeMap<Key, Arc<EventCatalog>>>>,
);

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
        self.event_pump.wake();
        Ok(())
    }
    pub(crate) fn is_dormant(&self, owner: &str, installation: &str) -> bool {
        self.workers
            .1
            .lock()
            .is_ok_and(|dormant| dormant.contains_key(&(owner.to_owned(), installation.to_owned())))
    }
    /// Recorded before an idle stop so tools stay discoverable while stopped.
    /// False when the bounded set is full; the caller then keeps the worker.
    pub(crate) fn retain_dormant(&self, owner: &str, catalog: Arc<EventCatalog>) -> bool {
        let Ok(mut dormant) = self.workers.1.lock() else {
            return false;
        };
        let key = (owner.to_owned(), catalog.installation_id().to_owned());
        if !dormant.contains_key(&key) && dormant.len() >= MAX_PROJECTIONS {
            return false;
        }
        dormant.insert(key, catalog);
        true
    }
    pub(crate) fn forget_dormant(&self, owner: &str, installation: &str) {
        if let Ok(mut dormant) = self.workers.1.lock() {
            dormant.remove(&(owner.to_owned(), installation.to_owned()));
        }
    }
}

impl AppControlService {
    /// The blocking lifecycle owner publishes only after the exact process's
    /// readiness and durable activation proof. It must retain AppWorkerOwner and
    /// serialize replacement for this installation through its complete drain.
    /// This registry provides discovery, not lifecycle admission or resource caps.
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
        if let Ok(mut dormant) = self.workers.1.lock() {
            dormant.remove(&(owner.to_owned(), installation.to_owned()));
        }
    }

    pub(crate) fn is_app_dormant(&self, owner: &str, installation: &str) -> bool {
        self.workers
            .1
            .lock()
            .is_ok_and(|dormant| dormant.contains_key(&(owner.to_owned(), installation.to_owned())))
    }

    /// Dormant (idle-stopped) catalogs for this owner, at most 64.
    pub(crate) fn dormant_app_catalogs(&self, owner: &str) -> Vec<Arc<EventCatalog>> {
        self.workers.1.lock().map_or_else(
            |_| Vec::new(),
            |dormant| {
                dormant
                    .iter()
                    .filter(|((key_owner, _), _)| key_owner == owner)
                    .map(|(_, catalog)| catalog.clone())
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
