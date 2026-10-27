use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(super) struct ProviderRunInFlightState {
    prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
    output_polls: Arc<Mutex<BTreeSet<String>>>,
}

impl ProviderRunInFlightState {
    pub(super) fn prompt_io_in_flight(&self, run_id: &str) -> bool {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .contains(run_id)
    }

    pub(super) fn mark_prompt_io_in_flight(&self, run_id: String) {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .insert(run_id);
    }

    pub(super) fn clear_prompt_io_in_flight(&self, run_id: &str) {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .remove(run_id);
    }

    #[cfg(test)]
    pub(super) fn output_poll_in_flight(&self, run_id: &str) -> bool {
        self.output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .contains(run_id)
    }

    pub(super) fn mark_output_poll_in_flight(&self, run_id: String) -> bool {
        self.output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .insert(run_id)
    }

    pub(super) fn clear_output_poll_in_flight(&self, run_id: &str) {
        self.output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .remove(run_id);
    }
}
