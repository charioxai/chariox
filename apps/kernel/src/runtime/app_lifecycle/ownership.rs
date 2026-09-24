//! Retained operation gates, cancellation and actual thread join ownership.
use super::*;
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.inner
            .operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}
impl Control {
    pub(super) fn new() -> Self {
        Self {
            first_request: None,
            stop: AtomicBool::new(false),
            manual: AtomicBool::new(false),
            manual_committed: AtomicBool::new(false),
            done: Mutex::new(false),
            wake: Condvar::new(),
            drain: Mutex::new(None),
        }
    }
    pub(super) fn cancel(&self, manual: bool) {
        if manual {
            self.manual.store(true, Ordering::Release);
        }
        self.stop.store(true, Ordering::Release);
        if let Some(drain) = self
            .drain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            drain.begin();
        }
        self.wake.notify_all();
    }
    pub(super) fn retain_drain(&self, drain: crate::runtime::app_worker::AppWorkerDrain) {
        let mut retained = self
            .drain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.stopped() {
            drain.begin();
        }
        *retained = Some(drain);
    }
    pub(super) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    pub(super) fn pending_manual_stop(&self) -> bool {
        self.manual.load(Ordering::Acquire) && !self.manual_committed.load(Ordering::Acquire)
    }
    pub(super) fn confirm_manual_stop(&self) {
        self.manual_committed.store(true, Ordering::Release);
    }
    pub(super) fn budget(self: &Arc<Self>) -> AppOperationBudget {
        let control = self.clone();
        AppOperationBudget::from_supervisor(move || control.stopped())
    }
    pub(super) fn finished(&self) -> bool {
        *self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn complete(&self) {
        *self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.wake.notify_all();
    }
    pub(super) fn wait(&self, duration: Duration) {
        let done = self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*done && !self.stopped() {
            let _ = self.wake.wait_timeout(done, duration);
        }
    }
}
impl Entry {
    pub(super) fn join(&self) {
        if let Some(thread) = self
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = thread.join();
            self.control.complete();
        } else {
            let mut done = self
                .control
                .done
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while !*done {
                done = self
                    .control
                    .wake
                    .wait(done)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
    }
}

impl Inner {
    pub(super) fn operation(&self, key: Key) -> Result<Operation<'_>> {
        let mut active = self
            .operations
            .lock()
            .map_err(|_| LifecycleError::Supervisor)?;
        if active.len() >= 8 || !active.insert(key.clone()) {
            return Err(LifecycleError::Busy);
        }
        Ok(Operation { inner: self, key })
    }
    pub(super) fn shutdown(&self) -> Result<()> {
        self.stopped.store(true, Ordering::Release);
        let _shutdown = self
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = std::mem::take(
            &mut *self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for entry in entries.values() {
            entry.control.cancel(false);
        }
        let mut pending = BTreeMap::new();
        for ((owner, installation), entry) in &entries {
            entry.join();
            // A requested user stop is distinct from ordinary kernel shutdown.
            // Make its final pending writer attempt after every native reap.
            if manual_stop::persist(
                &self.store,
                owner,
                installation,
                &entry.control,
                AppOperationBudget::from_supervisor(|| false),
            )
            .is_err()
            {
                pending.insert((owner.clone(), installation.clone()), entry.clone());
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        // Keep failed intent owned for an explicit retry. A caller must see
        // failed shutdown rather than confirmation of a stop we could not save.
        *self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = pending;
        Err(LifecycleError::Storage)
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
