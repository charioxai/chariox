//! One kernel-owned pool. Stopping a stream does not release its reservation;
//! the final socket/driver owner must actually drop the retained lease.
use super::{HttpError, Result, MAX_STREAMS_PER_INSTALLATION, MAX_STREAMS_PER_KERNEL};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(crate) struct HttpLimits(Arc<Mutex<Counts>>);
#[derive(Default)]
struct Counts {
    total: usize,
    installations: BTreeMap<(String, String), usize>,
}
#[derive(Clone)]
pub(super) struct LifetimeLease {
    // Dropped before capacity accounting. DNS driver panic/abort fallbacks must
    // retain the actual worker preparation until their I/O futures are gone.
    worker: Option<chariox_app_runtime::worker_process::PrivateData>,
    reservation: Arc<Reservation>,
}
struct Reservation {
    counts: Arc<Mutex<Counts>>,
    key: (String, String),
}

impl HttpLimits {
    /// Internal trusted scope, never an owner or installation supplied in an
    /// SDK request. This grants capacity only; it cannot authorize a request.
    pub(super) fn acquire(&self, owner: &str, installation: &str) -> Result<LifetimeLease> {
        let key = (owner.to_owned(), installation.to_owned());
        let mut counts = self.0.lock().map_err(|_| HttpError::Busy)?;
        if counts.total >= MAX_STREAMS_PER_KERNEL
            || counts.installations.get(&key).copied().unwrap_or(0) >= MAX_STREAMS_PER_INSTALLATION
        {
            return Err(HttpError::Busy);
        }
        counts.total += 1;
        *counts.installations.entry(key.clone()).or_default() += 1;
        Ok(LifetimeLease {
            worker: None,
            reservation: Arc::new(Reservation {
                counts: self.0.clone(),
                key,
            }),
        })
    }
}
impl LifetimeLease {
    pub(super) fn retain_worker(
        mut self,
        data: chariox_app_runtime::worker_process::PrivateData,
    ) -> Result<Self> {
        if data.installation_id() != self.reservation.key.1 {
            return Err(HttpError::Provenance);
        }
        self.worker = Some(data);
        Ok(self)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let Ok(mut counts) = self.counts.lock() else {
            return;
        };
        counts.total -= 1;
        if let Some(count) = counts.installations.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                counts.installations.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_streams_hold_capacity_until_all_io_owners_drop() {
        let limits = HttpLimits::default();
        let mut live = (0..4)
            .map(|_| limits.acquire("alice", "app").unwrap())
            .collect::<Vec<_>>();
        let retained_socket = live.pop().unwrap();
        let driver = retained_socket.clone();
        drop(retained_socket);
        assert!(matches!(
            limits.acquire("alice", "app"),
            Err(HttpError::Busy)
        ));
        drop(driver);
        live.push(limits.acquire("alice", "app").unwrap());
        let mut others = Vec::new();
        for i in 0..28 {
            others.push(limits.acquire("alice", &format!("app{i}")).unwrap());
        }
        assert!(matches!(
            limits.acquire("bob", "different"),
            Err(HttpError::Busy)
        ));
        drop(others.pop());
        assert!(limits.acquire("bob", "different").is_ok());
        drop(live);
        drop(others);
        assert_eq!(limits.0.lock().unwrap().total, 0);
        assert!(limits.0.lock().unwrap().installations.is_empty());
    }
}
