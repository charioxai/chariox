use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(super) struct ProviderRunInFlightState {
    // Per-run count of queued or running prompt-class commands (submit, utility, abort):
    // a second command queued behind the first must not vanish when the first completes.
    prompt_submissions: Arc<Mutex<std::collections::BTreeMap<String, usize>>>,
    output_polls: Arc<Mutex<BTreeSet<String>>>,
    plan_publications: Arc<Mutex<std::collections::BTreeMap<String, usize>>>,
}

impl ProviderRunInFlightState {
    pub(super) fn prompt_io_in_flight(&self, run_id: &str) -> bool {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .contains_key(run_id)
    }

    /// Runs with a prompt, steer or abort queued or being written.
    pub(super) fn prompt_io_in_flight_runs(&self) -> Vec<String> {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .keys()
            .cloned()
            .collect()
    }

    pub(super) fn mark_prompt_io_in_flight(&self, run_id: String) {
        *self
            .prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .entry(run_id)
            .or_default() += 1;
    }

    /// One command finished: the run stays in flight while another is queued.
    pub(super) fn clear_prompt_io_in_flight(&self, run_id: &str) {
        let mut runs = self
            .prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned");
        if let Some(count) = runs.get_mut(run_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                runs.remove(run_id);
            }
        }
    }

    /// The run's runtime is gone: nothing queued for it counts any more.
    pub(super) fn reset_prompt_io_in_flight(&self, run_id: &str) {
        self.prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .remove(run_id);
    }

    /// Runs with an output poll queued or running.
    pub(super) fn output_poll_in_flight_runs(&self) -> Vec<String> {
        self.output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn mark_plan_publication_in_flight(&self, run_id: String) {
        *self
            .plan_publications
            .lock()
            .expect("plan publication map poisoned")
            .entry(run_id)
            .or_default() += 1;
    }

    pub(super) fn clear_plan_publication_in_flight(&self, run_id: &str) {
        let mut plans = self
            .plan_publications
            .lock()
            .expect("plan publication map poisoned");
        if let Some(count) = plans.get_mut(run_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                plans.remove(run_id);
            }
        }
    }

    pub(super) fn plan_publication_in_flight_runs(&self) -> Vec<String> {
        self.plan_publications
            .lock()
            .expect("plan publication map poisoned")
            .keys()
            .cloned()
            .collect()
    }

    pub(super) fn any_output_poll_in_flight(&self) -> bool {
        !self
            .output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .is_empty()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_queued_command_keeps_the_run_in_flight_when_the_first_completes() {
        let state = ProviderRunInFlightState::default();
        state.mark_prompt_io_in_flight("run".to_string());
        state.mark_prompt_io_in_flight("run".to_string());
        state.clear_prompt_io_in_flight("run");
        assert!(
            state.prompt_io_in_flight("run"),
            "the second command is still queued"
        );
        assert_eq!(state.prompt_io_in_flight_runs(), vec!["run".to_string()]);
        state.clear_prompt_io_in_flight("run");
        assert!(!state.prompt_io_in_flight("run"));
        assert!(state.prompt_io_in_flight_runs().is_empty());
    }

    #[test]
    fn a_gone_runtime_resets_every_queued_command_of_its_run() {
        let state = ProviderRunInFlightState::default();
        state.mark_prompt_io_in_flight("run".to_string());
        state.mark_prompt_io_in_flight("run".to_string());
        state.reset_prompt_io_in_flight("run");
        assert!(!state.prompt_io_in_flight("run"));
    }

    #[test]
    fn overlapping_plan_publications_are_counted_per_command() {
        let state = ProviderRunInFlightState::default();
        state.mark_plan_publication_in_flight("run".to_string());
        state.mark_plan_publication_in_flight("run".to_string());
        state.clear_plan_publication_in_flight("run");
        assert_eq!(
            state.plan_publication_in_flight_runs(),
            vec!["run".to_string()]
        );
        state.clear_plan_publication_in_flight("run");
        assert!(state.plan_publication_in_flight_runs().is_empty());
    }
}
