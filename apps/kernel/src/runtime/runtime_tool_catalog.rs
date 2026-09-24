//! Bounded cache invalidation for the existing runtime MCP, never tool authority.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

const MAX_STREAMS: usize = 256;
const MAX_STREAMS_PER_RUN: usize = 4;
const MAX_REFRESHES: usize = 8;

#[derive(Clone, Default)]
pub(crate) struct RuntimeToolCatalogChanges(Arc<Mutex<Registry>>);
#[derive(Default)]
struct Registry {
    entries: BTreeMap<String, Entry>,
    next: u64,
    streams: usize,
    refreshes: usize,
}
struct Entry {
    identity: u64,
    signal: watch::Sender<CatalogRevision>,
    streams: usize,
    refreshing: bool,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CatalogRevision {
    pub(crate) desired: u64,
    pub(crate) observed: u64,
}
pub(crate) struct CatalogWatch {
    changes: RuntimeToolCatalogChanges,
    run_id: String,
    identity: u64,
    refresh: bool,
    receiver: watch::Receiver<CatalogRevision>,
}

impl RuntimeToolCatalogChanges {
    pub(crate) fn subscribe(&self, run_id: &str) -> Option<CatalogWatch> {
        self.acquire(run_id, false)
    }
    pub(crate) fn begin_refresh(&self, run_id: &str) -> Option<CatalogWatch> {
        let watch = self.acquire(run_id, true)?;
        self.invalidate(run_id);
        Some(watch)
    }
    fn acquire(&self, run_id: &str, refresh: bool) -> Option<CatalogWatch> {
        let mut registry = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if (refresh && registry.refreshes >= MAX_REFRESHES)
            || (!refresh && registry.streams >= MAX_STREAMS)
        {
            return None;
        }
        if !registry.entries.contains_key(run_id) {
            registry.next = registry.next.checked_add(1)?;
            let identity = registry.next;
            registry.entries.insert(
                run_id.into(),
                Entry {
                    identity,
                    signal: watch::channel(CatalogRevision::default()).0,
                    streams: 0,
                    refreshing: false,
                },
            );
        }
        let entry = registry.entries.get_mut(run_id)?;
        if (refresh && entry.refreshing) || (!refresh && entry.streams >= MAX_STREAMS_PER_RUN) {
            return None;
        }
        let identity = entry.identity;
        let receiver = entry.signal.subscribe();
        if refresh {
            entry.refreshing = true;
        } else {
            entry.streams += 1;
        }
        if refresh {
            registry.refreshes += 1;
        } else {
            registry.streams += 1;
        }
        Some(CatalogWatch {
            changes: self.clone(),
            run_id: run_id.into(),
            identity,
            refresh,
            receiver,
        })
    }
    pub(crate) fn invalidate(&self, run_id: &str) {
        let mut registry = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = registry.entries.get_mut(run_id) else {
            return;
        };
        entry.signal.send_modify(|revision| {
            // Revision equality is cache freshness only. Exhaustion closes the
            // run's subscribers instead of reusing a previously observed value.
            revision.desired = revision.desired.saturating_add(1);
        });
        let exhausted = entry.signal.borrow().desired == u64::MAX;
        if exhausted {
            drop(registry);
            self.close(run_id);
        }
    }
    pub(crate) fn revision(&self, run_id: &str) -> Option<(u64, u64)> {
        let registry = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = registry.entries.get(run_id)?;
        let revision = entry.signal.borrow().desired;
        Some((entry.identity, revision))
    }
    pub(crate) fn observed(&self, run_id: &str, captured: (u64, u64)) {
        let registry = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = registry.entries.get(run_id) else {
            return;
        };
        if entry.identity != captured.0 {
            return;
        }
        entry.signal.send_if_modified(|revision| {
            if revision.desired != captured.1 || revision.observed == captured.1 {
                return false;
            }
            revision.observed = captured.1;
            true
        });
    }
    pub(crate) fn close(&self, run_id: &str) {
        let mut registry = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Closing invalidates the run immediately, but a backpressured HTTP
        // body or blocking refresh can still own its watch. Keep its global
        // reservation until that actual owner drops, even after replacement.
        registry.entries.remove(run_id);
    }
}
impl CatalogWatch {
    /// Serialize the cache certificate with invalidation. The callback must
    /// only update local cache metadata and must not call this registry again.
    pub(crate) fn with_observed_current<T>(&self, confirm: impl FnOnce() -> T) -> Option<T> {
        let registry = self
            .changes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = registry.entries.get(&self.run_id)?;
        if entry.identity != self.identity {
            return None;
        }
        let revision = *entry.signal.borrow();
        if revision.observed != revision.desired {
            return None;
        }
        Some(confirm())
    }
    pub(crate) fn current(&self) -> CatalogRevision {
        *self.receiver.borrow()
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.receiver.has_changed().is_err()
    }
    pub(crate) async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.receiver.changed().await
    }
    pub(crate) async fn wait_until_observed(&mut self) -> Result<(), watch::error::RecvError> {
        loop {
            if let Err(error) = self.receiver.has_changed() {
                return Err(error);
            }
            let revision = self.current();
            if revision.observed == revision.desired {
                return Ok(());
            }
            self.changed().await?;
        }
    }
}
impl Drop for CatalogWatch {
    fn drop(&mut self) {
        let mut registry = self
            .changes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.refresh {
            registry.refreshes -= 1;
        } else {
            registry.streams -= 1;
        }
        let Some(entry) = registry.entries.get_mut(&self.run_id) else {
            return;
        };
        if entry.identity != self.identity {
            return;
        }
        if self.refresh {
            entry.refreshing = false;
        } else {
            entry.streams -= 1;
        }
        let unused = entry.streams == 0 && !entry.refreshing;
        if unused {
            registry.entries.remove(&self.run_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_discovery_cannot_acknowledge_a_newer_catalog() {
        let changes = RuntimeToolCatalogChanges::default();
        let mut refresh = changes.begin_refresh("run").unwrap();
        let old = changes.revision("run").unwrap();
        changes.invalidate("run");
        changes.observed("run", old);
        assert_ne!(refresh.current().desired, refresh.current().observed);
        changes.observed("run", changes.revision("run").unwrap());
        refresh.wait_until_observed().await.unwrap();
        assert_eq!(
            refresh.with_observed_current(|| "observed"),
            Some("observed")
        );
        changes.invalidate("run");
        assert_eq!(
            refresh.with_observed_current(|| panic!("stale callback")),
            None::<()>
        );
        changes.close("run");
        assert!(refresh.wait_until_observed().await.is_err());
    }

    #[test]
    fn backpressure_coalesces_and_stream_and_refresh_reservations_are_bounded() {
        let changes = RuntimeToolCatalogChanges::default();
        let streams: Vec<_> = (0..MAX_STREAMS_PER_RUN)
            .map(|_| changes.subscribe("run").unwrap())
            .collect();
        assert!(changes.subscribe("run").is_none());
        for _ in 0..10_000 {
            changes.invalidate("run");
        }
        assert_eq!(streams[0].current().desired, 10_000);
        assert_eq!(changes.0.lock().unwrap().entries.len(), 1);
        let refreshes: Vec<_> = (0..MAX_REFRESHES)
            .map(|id| changes.begin_refresh(&format!("refresh{id}")).unwrap())
            .collect();
        assert!(changes.begin_refresh("overflow").is_none());
        assert!(changes.begin_refresh("refresh0").is_none());
        drop(refreshes);
        drop(streams);
        assert!(changes.0.lock().unwrap().entries.is_empty());
        let global: Vec<_> = (0..MAX_STREAMS)
            .map(|id| changes.subscribe(&format!("run{id}")).unwrap())
            .collect();
        assert!(changes.subscribe("overflow").is_none());
        drop(global);
        assert!(changes.subscribe("recovered").is_some());
    }

    #[test]
    fn closing_a_run_cannot_invalidate_another_or_release_a_replacement_reservation() {
        let changes = RuntimeToolCatalogChanges::default();
        let first = changes.subscribe("one").unwrap();
        let second = changes.subscribe("two").unwrap();
        changes.invalidate("one");
        assert_eq!(second.current().desired, 0);
        changes.close("one");
        let replacement = changes.subscribe("one").unwrap();
        drop(first);
        assert!(!replacement.is_closed());
        assert_eq!(changes.0.lock().unwrap().streams, 2);
    }

    #[test]
    fn closed_watches_retain_global_capacity_until_actual_owner_drop() {
        let changes = RuntimeToolCatalogChanges::default();
        let streams: Vec<_> = (0..MAX_STREAMS)
            .map(|index| {
                let id = format!("stream{index}");
                let watch = changes.subscribe(&id).unwrap();
                changes.close(&id);
                assert!(watch.is_closed());
                watch
            })
            .collect();
        let refreshes: Vec<_> = (0..MAX_REFRESHES)
            .map(|index| {
                let id = format!("refresh{index}");
                let watch = changes.begin_refresh(&id).unwrap();
                changes.close(&id);
                assert!(watch.is_closed());
                watch
            })
            .collect();
        assert!(changes.0.lock().unwrap().entries.is_empty());
        assert!(changes.subscribe("stream0").is_none());
        assert!(changes.begin_refresh("refresh0").is_none());
        drop(streams);
        drop(refreshes);
        let stream = changes.subscribe("stream0").unwrap();
        let refresh = changes.begin_refresh("refresh0").unwrap();
        assert!(!stream.is_closed());
        assert!(!refresh.is_closed());
    }
}
