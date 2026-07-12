use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::provider::ProviderPromptSignalBatch;
use crate::terminal::TerminalOutputRecord;

pub(crate) const STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS: u64 = 500;

#[derive(Clone, Default)]
pub(crate) struct StructuredOutputRecordStore {
    records: Arc<Mutex<BTreeMap<String, Vec<TerminalOutputRecord>>>>,
    next_poll_due_at_ms: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl StructuredOutputRecordStore {
    pub(crate) fn take(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id)
            .unwrap_or_default()
    }

    pub(crate) fn append(&self, provider_run_id: String, records: Vec<TerminalOutputRecord>) {
        if records.is_empty() {
            return;
        }
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .entry(provider_run_id)
            .or_default()
            .extend(records);
    }

    pub(crate) fn poll_due(&self, provider_run_id: &str, now_ms: u64) -> bool {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .get(provider_run_id)
            .is_none_or(|due_at_ms| *due_at_ms <= now_ms)
    }

    pub(crate) fn mark_poll_enqueued(&self, provider_run_id: &str) {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
    }

    pub(crate) fn schedule_next_poll(&self, provider_run_id: String, due_at_ms: u64) {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .insert(provider_run_id, due_at_ms);
    }

    #[cfg(test)]
    pub(crate) fn poll_due_at_ms(&self, provider_run_id: &str) -> Option<u64> {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .get(provider_run_id)
            .copied()
    }

    pub(crate) fn take_due_provider_run_ids(&self, now_ms: u64) -> BTreeSet<String> {
        let mut schedule = self
            .next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned");
        let due = schedule
            .iter()
            .filter(|(_, due_at_ms)| **due_at_ms <= now_ms)
            .map(|(provider_run_id, _)| provider_run_id.clone())
            .collect::<BTreeSet<_>>();
        for provider_run_id in &due {
            schedule.remove(provider_run_id);
        }
        due
    }

    pub(crate) fn next_poll_due_at_ms(&self) -> Option<u64> {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .values()
            .copied()
            .min()
    }

    pub(crate) fn clear(&self, provider_run_id: &str) {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id);
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
    }

    pub(crate) fn schedule_after_empty_poll(
        &self,
        provider_run_id: impl Into<String>,
        now_ms: u64,
    ) {
        self.schedule_next_poll(
            provider_run_id.into(),
            now_ms.saturating_add(STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS),
        );
    }
}

pub(crate) fn structured_output_batch_should_poll_immediately(
    batch: &ProviderPromptSignalBatch,
) -> bool {
    !batch.chunks.is_empty()
        || !batch.completions.is_empty()
        || batch.prompt_completed
        || batch.terminal_failure.is_some()
        || !batch.notices.is_empty()
}

#[cfg(test)]
mod tests {
    use super::StructuredOutputRecordStore;

    #[test]
    fn structured_output_poll_schedule_defers_empty_poll_reenqueue() {
        let store = StructuredOutputRecordStore::default();

        assert!(store.poll_due("provider-run-1", 1_000));

        store.schedule_next_poll("provider-run-1".to_string(), 1_500);

        assert!(!store.poll_due("provider-run-1", 1_499));
        assert!(store.poll_due("provider-run-1", 1_500));
        assert_eq!(store.poll_due_at_ms("provider-run-1"), Some(1_500));
        assert_eq!(store.next_poll_due_at_ms(), Some(1_500));
        assert!(store.take_due_provider_run_ids(1_499).is_empty());
        assert_eq!(
            store.take_due_provider_run_ids(1_500),
            ["provider-run-1".to_string()].into_iter().collect()
        );

        store.mark_poll_enqueued("provider-run-1");

        assert!(store.poll_due("provider-run-1", 1_501));
        assert_eq!(store.poll_due_at_ms("provider-run-1"), None);
    }
}
