//! Bounded response correlation for snapshot RPCs that outlive the ownership lock.

use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const MAX_PENDING_READS: usize = 64;
type Waiters<T> = BTreeMap<u64, mpsc::Sender<Result<T, String>>>;

pub(super) struct PendingResponses<T>(Arc<Mutex<Waiters<T>>>);

impl<T> Clone for PendingResponses<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for PendingResponses<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(BTreeMap::new())))
    }
}

impl<T> PendingResponses<T> {
    pub(super) fn register(&self, id: u64) -> Result<PendingResponse<T>, String> {
        let mut waiters = self
            .0
            .lock()
            .map_err(|_| "controller read registry poisoned")?;
        if waiters.len() >= MAX_PENDING_READS {
            return Err("browser controller has too many pending snapshot reads".into());
        }
        if waiters.contains_key(&id) {
            return Err("browser controller snapshot request ID already pending".into());
        }
        let (sender, receiver) = mpsc::channel();
        waiters.insert(id, sender);
        Ok(PendingResponse {
            id,
            receiver,
            registry: self.clone(),
        })
    }

    pub(super) fn is_empty(&self) -> Result<bool, String> {
        self.0
            .lock()
            .map(|waiters| waiters.is_empty())
            .map_err(|_| "controller read registry poisoned".into())
    }

    // Unknown IDs remain on the existing serial mutation/cancellation path.
    pub(super) fn route(&self, id: u64, response: T) -> Option<T> {
        let sender = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        match sender {
            Some(sender) => {
                let _ = sender.send(Ok(response));
                None
            }
            None => Some(response),
        }
    }

    pub(super) fn fail_all(&self, error: &str) {
        let waiters =
            std::mem::take(&mut *self.0.lock().unwrap_or_else(|error| error.into_inner()));
        for sender in waiters.into_values() {
            let _ = sender.send(Err(error.to_string()));
        }
    }
}

pub(super) struct PendingResponse<T> {
    id: u64,
    receiver: mpsc::Receiver<Result<T, String>>,
    registry: PendingResponses<T>,
}

impl<T> PendingResponse<T> {
    pub(super) fn wait(&self, timeout: Duration) -> Result<T, String> {
        self.receiver
            .recv_timeout(timeout)
            .map_err(|error| format!("browser controller snapshot response unavailable: {error}"))?
    }
}

impl<T> Drop for PendingResponse<T> {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_receive_their_own_out_of_order_replies() {
        let registry = PendingResponses::default();
        let first = registry.register(1).unwrap();
        let second = registry.register(2).unwrap();
        assert!(!registry.is_empty().unwrap());
        assert_eq!(registry.route(2, "second"), None);
        assert_eq!(registry.route(1, "first"), None);
        assert_eq!(first.wait(Duration::ZERO).unwrap(), "first");
        assert_eq!(second.wait(Duration::ZERO).unwrap(), "second");
        assert_eq!(registry.route(3, "mutation"), Some("mutation"));
        assert!(registry.is_empty().unwrap());
    }

    #[test]
    fn independent_waiters_do_not_hold_the_dispatch_registry_lock() {
        let registry = PendingResponses::default();
        let first = registry.register(1).unwrap();
        let second = registry.register(2).unwrap();
        let first = std::thread::spawn(move || first.wait(Duration::from_secs(1)));
        let second = std::thread::spawn(move || second.wait(Duration::from_secs(1)));
        assert_eq!(registry.route(2, "second"), None);
        assert_eq!(registry.route(1, "first"), None);
        assert_eq!(second.join().unwrap().unwrap(), "second");
        assert_eq!(first.join().unwrap().unwrap(), "first");
    }

    #[test]
    fn dropped_and_timed_out_reads_release_the_bound() {
        let registry = PendingResponses::<()>::default();
        let mut pending = Vec::new();
        for id in 0..MAX_PENDING_READS as u64 {
            pending.push(registry.register(id).unwrap());
        }
        assert!(registry.register(100).is_err());
        assert!(pending[0].wait(Duration::ZERO).is_err());
        pending.remove(0);
        let replacement = registry.register(100).unwrap();
        drop(replacement);
        drop(pending);
        assert!(registry.is_empty().unwrap());
    }

    #[test]
    fn exit_or_invalid_json_fails_every_read_without_stealing_mutation_replies() {
        let registry = PendingResponses::<()>::default();
        let first = registry.register(1).unwrap();
        let second = registry.register(2).unwrap();
        assert!(registry.register(1).is_err());
        registry.fail_all("controller exited");
        assert_eq!(first.wait(Duration::ZERO).unwrap_err(), "controller exited");
        assert_eq!(
            second.wait(Duration::ZERO).unwrap_err(),
            "controller exited"
        );
        assert_eq!(registry.route(3, ()), Some(()));
        assert!(registry.is_empty().unwrap());
    }
}
