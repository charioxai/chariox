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
            stop: AtomicBool::new(false),
            manual: AtomicBool::new(false),
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
    pub(super) fn shutdown(&self) {
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
        for entry in entries.values() {
            entry.join();
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
    }
}
