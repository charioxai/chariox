//! Serialized internal start/stop actions; all authority stays on the writer.
use super::*;
impl AppLifecycleService {
    /// Internal kernel action only. No client-selected path, package metadata,
    /// runtime executable, permission decision or sandbox flag is accepted.
    pub(crate) fn start_active_blocking(
        &self,
        owner: &str,
        installation: &str,
        runtime: Handle,
    ) -> Result<StartDisposition> {
        self.start(owner, installation, false, runtime)
    }
    pub(super) fn start(
        &self,
        owner: &str,
        installation: &str,
        recovery: bool,
        runtime: Handle,
    ) -> Result<StartDisposition> {
        self.start_kind(owner, installation, StartKind::Active { recovery }, runtime)
    }
    pub(super) fn start_kind(
        &self,
        owner: &str,
        installation: &str,
        kind: StartKind,
        runtime: Handle,
    ) -> Result<StartDisposition> {
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(LifecycleError::Stopped);
        }
        for value in [owner, installation] {
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(LifecycleError::Authority);
            }
        }
        self.reap_finished();
        let key = (owner.to_owned(), installation.to_owned());
        let _operation_guard = self.0.operation(key.clone())?;
        let mut entries = self
            .0
            .entries
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?;
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(LifecycleError::Stopped);
        }
        if let Some(entry) = entries.get(&key) {
            return Ok(StartDisposition::Existing {
                attempt: entry.attempt.clone(),
            });
        }
        // Completed owners with uncommitted manual-stop intent retain an entry
        // until maintenance persists it. Bound those entries as well as workers.
        if entries.len() >= LIVE_LIMIT {
            return Err(LifecycleError::Busy);
        }
        let live = self
            .0
            .live
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::Busy)?;
        let preparation = self
            .0
            .preparation
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::Busy)?;
        let operation = self
            .0
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::Busy)?;
        let attempt = format!("{:032x}", rand::random::<u128>());
        let mut control = Control::new();
        if let StartKind::First { request_id } = &kind {
            control.first_request = Some(request_id.clone());
        }
        let control = Arc::new(control);
        let entry = Arc::new(Entry {
            attempt: attempt.clone(),
            control: control.clone(),
            thread: Mutex::new(None),
        });
        let context = owner::Context {
            store: self.0.store.clone(),
            publisher: self.0.publisher.clone(),
            admission: self.0.admission.clone(),
            owner: owner.into(),
            installation: installation.into(),
            attempt: attempt.clone(),
            control: control.clone(),
            runtime,
            kind,
            #[cfg(test)]
            fixture: self
                .0
                .fixture
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            #[cfg(test)]
            claim_checkpoint: self
                .0
                .claim_checkpoint
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        };
        // No thread retains Inner/AppControl. Its final owner can always cancel
        // and join every child, including when an awaiting caller disappears.
        let worker = std::thread::Builder::new()
            .name("chariox-app-owner".into())
            .spawn(move || owner::run(context, live, preparation, operation))
            .map_err(|_| LifecycleError::Supervisor)?;
        *entry
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);
        entries.insert(key, entry);
        Ok(StartDisposition::Starting { attempt })
    }
    pub(super) fn reap_finished(&self) {
        let finished = {
            let mut entries = self
                .0
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let keys: Vec<_> = entries
                .iter()
                .filter(|(_, entry)| {
                    entry.control.finished() && !entry.control.pending_manual_stop()
                })
                .map(|(key, _)| key.clone())
                .collect();
            keys.into_iter()
                .filter_map(|key| entries.remove(&key))
                .collect::<Vec<_>>()
        };
        for entry in finished {
            entry.join();
        }
    }
    pub(crate) fn stop_blocking(&self, owner: &str, installation: &str) -> Result<()> {
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(LifecycleError::Stopped);
        }
        for value in [owner, installation] {
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(LifecycleError::Authority);
            }
        }
        let key = (owner.into(), installation.into());
        let _operation_guard = self.0.operation(key.clone())?;
        let entry = self
            .0
            .entries
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?
            .get(&key)
            .cloned();
        if let Some(entry) = &entry {
            entry.control.cancel(true);
        }
        // Stop admission is not held hostage by the worker's existing calls.
        // On saturation the owner still drains and records its manual-stop
        // intent; Busy only means this caller cannot join a writer operation.
        let _permit = self
            .0
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::Busy)?;
        // Durable stop intent fences a recovery candidate read before this call.
        let budget = AppOperationBudget::from_supervisor(|| false);
        let result: Result<()> = (|| {
            if let Some(entry) = &entry {
                manual_stop::cancel_first(
                    &self.0.store,
                    owner,
                    &entry.control,
                    budget.fork(|| false),
                )?;
            }
            self.0
                .store
                .stop_app_worker_intent(owner, installation, budget)
                .map_err(Into::into)
        })();
        let entry = self
            .0
            .entries
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?
            .get(&key)
            .cloned();
        if let Some(entry) = entry {
            entry.control.cancel(true);
            if result.is_ok() {
                entry.control.confirm_manual_stop();
            }
            entry.join();
            let mut entries = self
                .0
                .entries
                .lock()
                .map_err(|_| LifecycleError::Supervisor)?;
            if entries.get(&key).is_some_and(|current| {
                Arc::ptr_eq(current, &entry) && !entry.control.pending_manual_stop()
            }) {
                entries.remove(&key);
            }
        }
        result?;
        self.0
            .store
            .finish_app_worker_stop(
                owner,
                installation,
                AppOperationBudget::from_supervisor(|| false),
            )
            .map_err(Into::into)
    }
    /// Must be called from bounded blocking shutdown ownership before runtime
    /// teardown. A Drop fallback retains the same no-orphan guarantee.
    pub(crate) fn shutdown_blocking(&self) -> Result<()> {
        self.0.shutdown()
    }
}
