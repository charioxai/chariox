//! Bounded discovery of actual activated process owners. These weak projections
//! never keep a process alive or grant an agent permission to invoke its tools.
use super::AppControlService;
use crate::runtime::app_worker::{ActivatedApp, AppWorkerError, AppWorkerLease};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const MAX_PROJECTIONS: usize = 64;
type Key = (String, String);
#[derive(Clone, Default)]
pub(super) struct ActiveWorkers(Arc<Mutex<BTreeMap<Key, ActivatedApp>>>);

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
        workers.insert(key, worker);
        drop(workers);
        self.event_pump.wake();
        Ok(())
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
