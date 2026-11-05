//! Cancellation of fresh metadata children without blocking the run actor's Abort.
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Default)]
pub(super) struct MetadataUtilityCancellations(Arc<Mutex<BTreeMap<String, Arc<AtomicU64>>>>);

#[derive(Clone)]
pub(super) struct MetadataUtilityCancellation {
    epoch: Arc<AtomicU64>,
    admitted_epoch: u64,
    /// The Stop of the one request this utility answers.
    request: Option<crate::provider::UtilityStop>,
}

impl MetadataUtilityCancellations {
    pub(super) fn for_run(
        &self,
        run_id: &str,
        request: Option<crate::provider::UtilityStop>,
    ) -> MetadataUtilityCancellation {
        let epoch = self
            .0
            .lock()
            .expect("metadata utility cancellation mutex poisoned")
            .entry(run_id.to_string())
            .or_default()
            .clone();
        let admitted_epoch = epoch.load(Ordering::Acquire);
        MetadataUtilityCancellation {
            epoch,
            admitted_epoch,
            request,
        }
    }

    pub(super) fn cancel_and_forget(&self, run_id: &str) {
        if let Some(epoch) = self
            .0
            .lock()
            .expect("metadata utility cancellation mutex poisoned")
            .remove(run_id)
        {
            epoch.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(super) fn cancel(&self, run_id: &str) {
        if let Some(epoch) = self
            .0
            .lock()
            .expect("metadata utility cancellation mutex poisoned")
            .get(run_id)
        {
            epoch.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl MetadataUtilityCancellation {
    pub(super) fn is_cancelled(&self) -> bool {
        self.epoch.load(Ordering::Acquire) != self.admitted_epoch
            || self.request.as_ref().is_some_and(|stop| stop.is_stopped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abort_cancels_prior_utilities_only_for_its_run() {
        let store = MetadataUtilityCancellations::default();
        let prior = store.for_run("one", None);
        let queued = store.for_run("one", None);
        let other = store.for_run("two", None);
        store.cancel("one");
        assert!(prior.is_cancelled() && queued.is_cancelled());
        assert!(!other.is_cancelled());
        let subsequent = store.for_run("one", None);
        assert!(!subsequent.is_cancelled());
        store.cancel_and_forget("one");
        assert!(subsequent.is_cancelled());
        assert!(!store.for_run("one", None).is_cancelled());
        assert!(!other.is_cancelled());
    }
}
