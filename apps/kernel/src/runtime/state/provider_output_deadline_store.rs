use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(super) struct ProviderOutputDeadlineStore {
    inner: Arc<Mutex<ProviderOutputDeadlineState>>,
}

#[derive(Debug, Default)]
struct ProviderOutputDeadlineState {
    next_generation: u64,
    current: BTreeMap<String, (u64, u64)>,
    deadlines: BinaryHeap<Reverse<(u64, u64, String)>>,
}

impl ProviderOutputDeadlineStore {
    pub(super) fn schedule(&self, provider_run_id: &str, due_at_ms: u64) {
        let mut state = self
            .inner
            .lock()
            .expect("provider output deadline store poisoned");
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        state
            .current
            .insert(provider_run_id.to_string(), (due_at_ms, generation));
        state.deadlines.push(Reverse((
            due_at_ms,
            generation,
            provider_run_id.to_string(),
        )));
    }

    pub(super) fn clear(&self, provider_run_id: &str) {
        self.inner
            .lock()
            .expect("provider output deadline store poisoned")
            .current
            .remove(provider_run_id);
    }

    pub(super) fn take_due_provider_run_ids(&self, now_ms: u64) -> BTreeSet<String> {
        let mut state = self
            .inner
            .lock()
            .expect("provider output deadline store poisoned");
        let mut due = BTreeSet::new();
        loop {
            let Some(Reverse((due_at_ms, generation, provider_run_id))) =
                state.deadlines.peek().cloned()
            else {
                break;
            };
            if due_at_ms > now_ms {
                break;
            }
            state.deadlines.pop();
            if state.current.get(&provider_run_id) == Some(&(due_at_ms, generation)) {
                state.current.remove(&provider_run_id);
                due.insert(provider_run_id);
            }
        }
        due
    }

    pub(super) fn next_due_at_ms(&self) -> Option<u64> {
        let mut state = self
            .inner
            .lock()
            .expect("provider output deadline store poisoned");
        loop {
            let Reverse((due_at_ms, generation, provider_run_id)) = state.deadlines.peek()?.clone();
            if state.current.get(&provider_run_id) == Some(&(due_at_ms, generation)) {
                return Some(due_at_ms);
            }
            state.deadlines.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderOutputDeadlineStore;

    #[test]
    fn deadline_store_deduplicates_reschedules_and_discards_stale_heap_entries() {
        let store = ProviderOutputDeadlineStore::default();
        store.schedule("run-1", 1_000);
        store.schedule("run-1", 2_000);
        store.schedule("run-2", 1_500);

        assert_eq!(store.next_due_at_ms(), Some(1_500));
        assert!(store.take_due_provider_run_ids(1_499).is_empty());
        assert_eq!(
            store.take_due_provider_run_ids(1_500),
            ["run-2".to_string()].into_iter().collect()
        );
        assert_eq!(store.next_due_at_ms(), Some(2_000));
        store.clear("run-1");
        assert_eq!(store.next_due_at_ms(), None);
    }
}
