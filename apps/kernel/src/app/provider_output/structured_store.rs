use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::provider::ProviderPromptSignalBatch;
use crate::terminal::TerminalOutputRecord;

pub(crate) const STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS: u64 = 500;
pub(crate) const STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT: u8 = 3;

#[derive(Debug, Clone)]
struct PollFailureState {
    attempts: u8,
    prompt_id: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct StructuredOutputRecordStore {
    records: Arc<Mutex<BTreeMap<String, Vec<TerminalOutputRecord>>>>,
    output_ready: Arc<Mutex<BTreeSet<String>>>,
    next_poll_due_at_ms: Arc<Mutex<BTreeMap<String, u64>>>,
    in_flight_prompt_ids: Arc<Mutex<BTreeMap<String, Option<String>>>>,
    consecutive_poll_failures: Arc<Mutex<BTreeMap<String, PollFailureState>>>,
}

impl StructuredOutputRecordStore {
    pub(crate) fn take(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id)
            .unwrap_or_default()
    }

    pub(crate) fn take_and_stop_polling(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        let records = self.take(provider_run_id);
        self.stop_polling(provider_run_id);
        records
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

    pub(crate) fn mark_output_ready(&self, run_id: String) {
        self.output_ready
            .lock()
            .expect("structured output readiness poisoned")
            .insert(run_id);
    }

    pub(crate) fn poll_due(&self, provider_run_id: &str, now_ms: u64) -> bool {
        let failed_attempts = self
            .consecutive_poll_failures
            .lock()
            .expect("structured output poll failure map poisoned")
            .get(provider_run_id)
            .map(|state| state.attempts);
        if failed_attempts
            .is_some_and(|attempts| attempts >= STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT)
        {
            return false;
        }
        if self
            .in_flight_prompt_ids
            .lock()
            .expect("structured output poll prompt map poisoned")
            .contains_key(provider_run_id)
        {
            return false;
        }
        // Readiness is only a hint. A failed drain retains its retry delay
        // even if the peer keeps the socket readable (including EOF).
        if failed_attempts.is_some() {
            return self
                .next_poll_due_at_ms
                .lock()
                .expect("structured output poll schedule poisoned")
                .get(provider_run_id)
                .is_none_or(|due_at_ms| *due_at_ms <= now_ms);
        }
        self.output_ready
            .lock()
            .expect("structured output readiness poisoned")
            .contains(provider_run_id)
            || self
                .next_poll_due_at_ms
                .lock()
                .expect("structured output poll schedule poisoned")
                .get(provider_run_id)
                .is_none_or(|due_at_ms| *due_at_ms <= now_ms)
    }

    pub(crate) fn enqueue_poll<E>(
        &self,
        provider_run_id: &str,
        prompt_id: Option<String>,
        enqueue: impl FnOnce() -> Result<bool, E>,
    ) -> Result<bool, E> {
        // MP-10: publish the prompt fence before the actor job. A second
        // client may consume its completion before enqueue returns.
        {
            let mut prompt_ids = self
                .in_flight_prompt_ids
                .lock()
                .expect("structured output poll prompt map poisoned");
            if prompt_ids.contains_key(provider_run_id) {
                return Ok(false);
            }
            prompt_ids.insert(provider_run_id.to_string(), prompt_id);
        }
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
        self.output_ready
            .lock()
            .expect("structured output readiness poisoned")
            .remove(provider_run_id);
        let result = enqueue();
        if !matches!(&result, Ok(true)) {
            self.mark_output_ready(provider_run_id.to_string());
            self.take_in_flight_prompt_id(provider_run_id);
        }
        result
    }

    pub(crate) fn mark_poll_enqueued(&self, provider_run_id: &str, prompt_id: Option<String>) {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
        let mut prompt_ids = self
            .in_flight_prompt_ids
            .lock()
            .expect("structured output poll prompt map poisoned");
        prompt_ids.insert(provider_run_id.to_string(), prompt_id);
    }

    pub(crate) fn take_in_flight_prompt_id(&self, provider_run_id: &str) -> Option<String> {
        self.in_flight_prompt_ids
            .lock()
            .expect("structured output poll prompt map poisoned")
            .remove(provider_run_id)
            .flatten()
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
        self.stop_polling(provider_run_id);
        self.in_flight_prompt_ids
            .lock()
            .expect("structured output poll prompt map poisoned")
            .remove(provider_run_id);
        self.clear_poll_failures(provider_run_id);
    }

    pub(crate) fn stop_polling(&self, provider_run_id: &str) {
        self.output_ready
            .lock()
            .expect("structured output readiness poisoned")
            .remove(provider_run_id);
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
        self.clear_poll_failures(provider_run_id);
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

    pub(crate) fn schedule_after_stale_poll_failure(&self, provider_run_id: &str, now_ms: u64) {
        self.clear_poll_failures(provider_run_id);
        self.schedule_after_empty_poll(provider_run_id.to_string(), now_ms);
    }

    pub(crate) fn reset_poll_failures_if_prompt_changed(
        &self,
        provider_run_id: &str,
        current_prompt_id: &str,
        now_ms: u64,
    ) -> bool {
        let mut failures = self
            .consecutive_poll_failures
            .lock()
            .expect("structured output poll failure map poisoned");
        let Some(state) = failures.get(provider_run_id) else {
            return false;
        };
        if state.prompt_id.as_deref() == Some(current_prompt_id) {
            return false;
        }
        failures.remove(provider_run_id);
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .insert(
                provider_run_id.to_string(),
                now_ms.saturating_add(STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS),
            );
        true
    }

    pub(crate) fn mark_poll_succeeded(&self, provider_run_id: &str) {
        self.clear_poll_failures(provider_run_id);
    }

    #[cfg(test)]
    pub(crate) fn schedule_after_poll_failure(
        &self,
        provider_run_id: &str,
        now_ms: u64,
    ) -> Option<u8> {
        self.schedule_after_poll_failure_for_prompt(provider_run_id, None, now_ms)
    }

    pub(crate) fn schedule_after_poll_failure_for_prompt(
        &self,
        provider_run_id: &str,
        polled_prompt_id: Option<&str>,
        now_ms: u64,
    ) -> Option<u8> {
        let attempt = {
            let mut failures = self
                .consecutive_poll_failures
                .lock()
                .expect("structured output poll failure map poisoned");
            let state = failures
                .entry(provider_run_id.to_string())
                .or_insert_with(|| PollFailureState {
                    attempts: 0,
                    prompt_id: polled_prompt_id.map(str::to_string),
                });
            if state.prompt_id.as_deref() != polled_prompt_id {
                state.attempts = 0;
                state.prompt_id = polled_prompt_id.map(str::to_string);
            }
            state.attempts = state.attempts.saturating_add(1);
            state.attempts
        };
        if attempt >= STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            self.next_poll_due_at_ms
                .lock()
                .expect("structured output poll schedule poisoned")
                .remove(provider_run_id);
            return None;
        }
        self.schedule_after_empty_poll(provider_run_id.to_string(), now_ms);
        Some(attempt)
    }

    fn clear_poll_failures(&self, provider_run_id: &str) {
        self.consecutive_poll_failures
            .lock()
            .expect("structured output poll failure map poisoned")
            .remove(provider_run_id);
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
    use super::{StructuredOutputRecordStore, STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT};

    // MP-08 / MP-10: reviewer #985 finding 4. A readable failed socket
    // must not consume its complete retry budget in one pump wake.
    #[test]
    fn panes_stream_review_readiness_respects_failure_backoff() {
        let store = StructuredOutputRecordStore::default();
        assert_eq!(
            store.schedule_after_poll_failure_for_prompt("run", Some("prompt"), 1_000),
            Some(1)
        );
        store.mark_output_ready("run".into());
        assert!(!store.poll_due("run", 1_001));
        assert!(!store.poll_due("run", 1_499));
        assert!(store.poll_due("run", 1_500));
        // The shared pump consumes the due entry before invoking the actor.
        assert!(store.take_due_provider_run_ids(1_500).contains("run"));
        assert!(store.poll_due("run", 1_500));
        store.mark_poll_succeeded("run");
        assert!(store.poll_due("run", 1_501));
    }

    #[test]
    fn mp10_poll_publication_keeps_prompt_fence_for_immediate_completion() {
        let store = StructuredOutputRecordStore::default();
        let mut consumed_prompt = None;
        assert!(store
            .enqueue_poll("run", Some("prompt".into()), || {
                // Another client can consume a finished actor job as soon as the
                // provider-store enqueue publishes it, before enqueue returns.
                consumed_prompt = store.take_in_flight_prompt_id("run");
                Ok::<_, ()>(true)
            })
            .unwrap());
        assert_eq!(consumed_prompt.as_deref(), Some("prompt"));
        assert!(
            store.poll_due("run", u64::MAX),
            "consumed poll must not leave a phantom reservation"
        );
    }

    #[test]
    fn mp10_poll_publication_reserves_idle_polls() {
        let store = StructuredOutputRecordStore::default();
        store
            .enqueue_poll("run", None, || Ok::<_, ()>(true))
            .unwrap();
        assert!(
            !store.poll_due("run", u64::MAX),
            "an idle poll is still in flight"
        );
        assert_eq!(store.take_in_flight_prompt_id("run"), None);
        assert!(store.poll_due("run", u64::MAX));
    }

    #[test]
    fn mp10_poll_publication_coalesces_concurrent_producers() {
        let store = StructuredOutputRecordStore::default();
        store
            .enqueue_poll("run", Some("prompt".into()), || {
                assert!(!store
                    .enqueue_poll("run", Some("prompt".into()), || -> Result<bool, ()> {
                        panic!("duplicate poll was published")
                    })
                    .unwrap_or(false));
                Ok::<_, ()>(true)
            })
            .unwrap();
        assert_eq!(
            store.take_in_flight_prompt_id("run").as_deref(),
            Some("prompt")
        );
    }

    #[test]
    fn mp10_poll_publication_rolls_back_rejected_and_failed_enqueue() {
        let store = StructuredOutputRecordStore::default();
        assert_eq!(
            store.enqueue_poll("run", Some("prompt".into()), || Ok::<_, ()>(false)),
            Ok(false)
        );
        assert!(store.poll_due("run", u64::MAX));
        assert_eq!(
            store.enqueue_poll("run", Some("prompt".into()), || Err::<bool, _>("failed")),
            Err("failed")
        );
        assert!(store.poll_due("run", u64::MAX));
    }

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

        store.mark_poll_enqueued("provider-run-1", Some("prompt-1".to_string()));

        assert!(
            !store.poll_due("provider-run-1", 1_501),
            "an in-flight poll must suppress duplicate provider requests"
        );
        assert_eq!(store.poll_due_at_ms("provider-run-1"), None);
        assert_eq!(
            store.take_in_flight_prompt_id("provider-run-1").as_deref(),
            Some("prompt-1")
        );
    }

    #[test]
    fn structured_output_poll_failures_are_bounded_and_reset_by_success() {
        let store = StructuredOutputRecordStore::default();

        for attempt in 1..STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            assert_eq!(
                store.schedule_after_poll_failure("provider-run-1", 1_000),
                Some(attempt),
            );
        }
        assert_eq!(
            store.schedule_after_poll_failure("provider-run-1", 1_000),
            None,
        );
        assert!(!store.poll_due("provider-run-1", u64::MAX));

        store.mark_poll_succeeded("provider-run-1");

        assert!(store.poll_due("provider-run-1", u64::MAX));
        assert_eq!(
            store.schedule_after_poll_failure("provider-run-1", 2_000),
            Some(1),
        );
    }

    #[test]
    fn stale_poll_failure_resets_budget_before_rescheduling() {
        let store = StructuredOutputRecordStore::default();

        for attempt in 1..=STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            let expected_attempt =
                (attempt < STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT).then_some(attempt);
            assert_eq!(
                store.schedule_after_poll_failure("provider-run-1", 1_000),
                expected_attempt,
            );
        }
        assert!(!store.poll_due("provider-run-1", u64::MAX));

        store.schedule_after_stale_poll_failure("provider-run-1", 2_000);

        assert!(!store.poll_due("provider-run-1", 2_499));
        assert!(store.poll_due("provider-run-1", 2_500));
        assert_eq!(
            store.schedule_after_poll_failure("provider-run-1", 3_000),
            Some(1),
            "a replacement prompt must receive a fresh failure budget"
        );
    }

    #[test]
    fn prompt_scoped_poll_failure_budget_resets_only_for_a_new_prompt() {
        let store = StructuredOutputRecordStore::default();

        for attempt in 1..=STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            let expected_attempt =
                (attempt < STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT).then_some(attempt);
            assert_eq!(
                store.schedule_after_poll_failure_for_prompt(
                    "provider-run-1",
                    Some("prompt-1"),
                    1_000,
                ),
                expected_attempt,
            );
        }
        assert!(!store.poll_due("provider-run-1", u64::MAX));
        assert!(
            !store.reset_poll_failures_if_prompt_changed("provider-run-1", "prompt-1", 2_000),
            "re-observing the same prompt must not reset its exhausted budget"
        );
        assert!(!store.poll_due("provider-run-1", u64::MAX));

        assert!(store.reset_poll_failures_if_prompt_changed("provider-run-1", "prompt-2", 2_000,));
        assert!(!store.poll_due("provider-run-1", 2_499));
        assert!(store.poll_due("provider-run-1", 2_500));
        assert_eq!(
            store
                .schedule_after_poll_failure_for_prompt("provider-run-1", Some("prompt-2"), 3_000,),
            Some(1),
        );
    }
}

#[cfg(test)]
mod output_ready_tests {
    use super::StructuredOutputRecordStore;
    #[test]
    fn provider_arrival_survives_an_in_flight_empty_poll_backoff() {
        let store = StructuredOutputRecordStore::default();
        store.mark_poll_enqueued("run", Some("prompt".into()));
        store.mark_output_ready("run".into());
        assert!(!store.poll_due("run", 1000));
        store.take_in_flight_prompt_id("run");
        store.schedule_after_empty_poll("run", 1000);
        assert!(
            store.poll_due("run", 1001),
            "an event bypasses the 500ms fallback"
        );
        assert!(store
            .enqueue_poll("run", Some("prompt".into()), || Ok::<_, ()>(true))
            .unwrap());
        store.take_in_flight_prompt_id("run");
        store.schedule_after_empty_poll("run", 1001);
        assert!(
            !store.poll_due("run", 1002),
            "the next empty poll retains fallback backoff"
        );
        store.mark_output_ready("run".into());
        store.clear("run");
        store.schedule_after_empty_poll("run", 1002);
        assert!(
            !store.poll_due("run", 1003),
            "clear removes stale readiness"
        );
    }
}
