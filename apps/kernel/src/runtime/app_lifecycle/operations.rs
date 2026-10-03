//! Serialized internal start/stop actions; all authority stays on the writer.
use super::*;
impl AppLifecycleService {
    /// An accepted owner spans admission, the durable claim, and publication.
    /// A user stop or a finished owner must never make a call wait for restart.
    pub(crate) fn has_pending_owner(&self, owner: &str, installation: &str) -> bool {
        !self.0.stopped.load(Ordering::Acquire)
            && self
                .0
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&(owner.to_owned(), installation.to_owned()))
                .is_some_and(|entry| !entry.control.stopped() && !entry.control.finished())
    }

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
    /// On-demand start for a wake, event or tool call. Its synchronous gate
    /// mirrors the start claim, so a user stop, a failed generation within its
    /// restart backoff or quarantined, a revoked publisher or an inactive
    /// installation is never started by use.
    pub(crate) fn start_on_demand_blocking(
        &self,
        owner: &str,
        installation: &str,
        runtime: Handle,
    ) -> Result<StartDisposition> {
        use crate::durable_state::app_worker_lifecycle::StartGate;
        match self.0.store.app_worker_start_gate(owner, installation)? {
            StartGate::Allowed => self.start(owner, installation, true, runtime),
            StartGate::UserStopped => Err(LifecycleError::Stopped),
            StartGate::RestartDeferred => Err(LifecycleError::RestartDeferred),
            StartGate::Refused => Err(LifecycleError::Authority),
        }
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
        if let Some(entry) = entries.get(&key).cloned() {
            // Pending manual stop and queued suspension both exclude starts.
            if entry.control.pending_manual_stop()
                || entry.control.idle_requested.load(Ordering::Acquire)
            {
                return Err(LifecycleError::Busy);
            }
            let replacing = match &kind {
                StartKind::First {
                    request_id,
                    replace,
                } => *replace && entry.control.first_request.as_ref() != Some(request_id),
                StartKind::Active { .. } => false,
            };
            let retiring =
                entry.control.retiring.load(Ordering::Acquire) || entry.control.finished();
            if !replacing && !retiring {
                return Ok(StartDisposition::Existing {
                    attempt: entry.attempt.clone(),
                });
            }
            // Join a retiring owner or drain a local update's old generation
            // under this installation's guard, so no start interleaves.
            // It is not a user stop: a failed update restarts the old
            // generation on demand.
            drop(entries);
            if replacing && !entry.control.finished() {
                // Preparation precedes the worker drain and update fence.
                match entry.control.notify("prepare_update", serde_json::json!({
                    "request_id": match &kind { StartKind::First { request_id, .. } => request_id, _ => unreachable!() },
                })) {
                    Ok(()) | Err(LifecycleError::NotificationNotDispatched) => {},
                    Err(error) => return Err(error),
                }
            }
            if replacing {
                entry.control.cancel_for_update();
            }
            entry.join();
            entries = self
                .0
                .entries
                .lock()
                .map_err(|_| LifecycleError::Supervisor)?;
            if entries
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, &entry))
            {
                entries.remove(&key);
            }
            if entries.contains_key(&key) {
                return Err(LifecycleError::Busy);
            }
        }
        // Completed owners with uncommitted manual-stop intent retain an entry
        // until maintenance persists it. Bound those entries as well as workers.
        if entries.len() >= LIVE_LIMIT {
            return Err(LifecycleError::LiveLimit);
        }
        // Only a full live-worker set refuses. An accepted start queues on
        // its owner thread for the preparation slot and a shared App
        // operation slot, so concurrent starts (recovery after a reboot) no
        // longer fail Busy while another App prepares.
        let live = self
            .0
            .live
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::LiveLimit)?;
        let attempt = format!("{:032x}", rand::random::<u128>());
        let mut control = Control::new();
        if let StartKind::First { request_id, .. } = &kind {
            control.first_request = Some(request_id.clone());
        }
        let control = Arc::new(control);
        let entry = Arc::new(Entry {
            attempt: attempt.clone(),
            control: control.clone(),
            thread: Mutex::new(None),
        });
        let context = owner::Context {
            http_limits: self.0.http_limits.clone(),
            event_config: self.0.event_config.clone(),
            store: self.0.store.clone(),
            publisher: self.0.publisher.clone(),
            admission: self.0.admission.clone(),
            preparation: self.0.preparation.clone(),
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
            start_checkpoint: self.0.start_checkpoint.lock().unwrap().clone(),
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
            .spawn(move || owner::run(context, live))
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
        self.stop_under_gate(owner, installation, key)
    }
    pub(super) fn stop_under_gate(&self, owner: &str, installation: &str, key: Key) -> Result<()> {
        // A manual stop ends on-demand use too; its tools leave the catalog.
        self.0.publisher.forget_dormant(owner, installation);
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
        // A tool listing between the first forget and the durable intent may
        // have seeded the catalog again; the intent now keeps it out.
        self.0.publisher.forget_dormant(owner, installation);
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
    /// Stop an idle worker while keeping its restart intent. Its verified
    /// catalog stays dormant so tools remain discoverable; the next tool call
    /// or wake starts it on demand. Suspension is persisted before stopping. `still_idle` is
    /// re-evaluated under the operation guard, so use that arrived after
    /// candidate selection keeps the worker running; callers also treat an App
    /// with undelivered events as busy, because delivery needs a live lease.
    /// An event emitted between that check and the join waits for the App's
    /// next tool call or wake (bounded by receipt expiry). Receipts held by a
    /// paused automation also keep the App live, as before idle stop existed.
    /// Queue at most one suspend on the retained owner and return immediately.
    /// Wake/inbox scans and eviction never wait on App-controlled latency.
    pub(crate) fn request_idle_stop_blocking(
        &self,
        owner: &str,
        catalog: Arc<chariox_app_runtime::app_outbox::EventCatalog>,
        still_idle: impl Fn() -> bool,
    ) -> Result<bool> {
        self.queue_idle_stop(owner, catalog, still_idle)
            .map(|request| request.is_some())
    }
    #[cfg(test)]
    pub(crate) fn idle_stop_blocking(
        &self,
        owner: &str,
        catalog: Arc<chariox_app_runtime::app_outbox::EventCatalog>,
        still_idle: impl Fn() -> bool,
    ) -> Result<()> {
        let Some((entry, receipt)) = self.queue_idle_stop(owner, catalog, still_idle)? else {
            return Ok(());
        };
        entry.control.wait_notification(receipt)?;
        entry.join();
        self.reap_finished();
        Ok(())
    }
    fn queue_idle_stop(
        &self,
        owner: &str,
        catalog: Arc<chariox_app_runtime::app_outbox::EventCatalog>,
        still_idle: impl Fn() -> bool,
    ) -> Result<Option<(Arc<Entry>, notifications::Receipt)>> {
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(LifecycleError::Stopped);
        }
        let key = (owner.to_owned(), catalog.installation_id().to_owned());
        let _operation_guard = self.0.operation(key.clone())?;
        let Some(entry) = self
            .0
            .entries
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?
            .get(&key)
            .cloned()
        else {
            return Ok(None);
        };
        if entry.control.stopped() || entry.control.pending_manual_stop() || !still_idle() {
            return Ok(None);
        }
        let receipt = entry
            .control
            .enqueue("suspend", serde_json::json!({"reason":"idle"}))?;
        Ok(Some((entry, receipt)))
    }
    /// Must be called from bounded blocking shutdown ownership before runtime
    /// teardown. A Drop fallback retains the same no-orphan guarantee.
    pub(crate) fn shutdown_blocking(&self) -> Result<()> {
        self.0.shutdown()
    }
}
