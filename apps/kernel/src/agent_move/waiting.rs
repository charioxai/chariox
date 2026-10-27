//! A Move requested while the agent's turn runs waits for the turn instead of interrupting it.
//! The wait lives as long as the request: it is not journaled, and a dropped request ends it.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct WaitingMoves(Arc<Mutex<Waits>>);

#[derive(Default)]
struct Waits {
    next: u64,
    by_key: BTreeMap<String, u64>,
}

/// Held by the request that waits; dropping it ends the wait.
pub(crate) struct MoveWait {
    moves: WaitingMoves,
    key: String,
    id: u64,
}

impl WaitingMoves {
    fn lock(&self) -> std::sync::MutexGuard<'_, Waits> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// None when this agent already has a Move waiting.
    pub(crate) fn begin(&self, key: String) -> Option<MoveWait> {
        let mut waits = self.lock();
        if waits.by_key.contains_key(&key) {
            return None;
        }
        waits.next += 1;
        let id = waits.next;
        waits.by_key.insert(key.clone(), id);
        Some(MoveWait {
            moves: self.clone(),
            key,
            id,
        })
    }

    /// True when a waiting Move was cancelled.
    pub(crate) fn cancel(&self, key: &str) -> bool {
        self.lock().by_key.remove(key).is_some()
    }
}

impl MoveWait {
    pub(crate) fn cancelled(&self) -> bool {
        self.moves.lock().by_key.get(&self.key) != Some(&self.id)
    }
}

impl Drop for MoveWait {
    fn drop(&mut self) {
        // A newer wait for the same agent keeps its own entry.
        let mut waits = self.moves.lock();
        if waits.by_key.get(&self.key) == Some(&self.id) {
            waits.by_key.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_is_unique_cancellable_and_ends_with_its_request() {
        let waiting = WaitingMoves::default();
        let wait = waiting.begin("home:room:agent".into()).unwrap();
        assert!(waiting.begin("home:room:agent".into()).is_none());
        assert!(!wait.cancelled());
        assert!(waiting.cancel("home:room:agent"));
        assert!(wait.cancelled());
        assert!(!waiting.cancel("home:room:agent"));
        let newer = waiting.begin("home:room:agent".into()).unwrap();
        drop(wait);
        assert!(
            !newer.cancelled(),
            "an ended wait must not cancel the one that replaced it"
        );
        drop(newer);
        assert!(waiting.begin("home:room:agent".into()).is_some());
    }
}
