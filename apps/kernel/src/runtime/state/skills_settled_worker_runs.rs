//! MP-08 / MP-10: the worker provider run each leased agent's last settled turn used.
use super::*;
use std::sync::Mutex as StdMutex;

/// The home learns which worker provider run took a turn only when the turn settles, and
/// the binding forgets it when the prompt completes. This remembers the last one so the
/// next turn can tell whether the run that holds earlier skill loads is still the run in
/// use. A restarted home remembers nothing, and a missing run re-attaches every load.
#[derive(Default)]
pub(super) struct SettledWorkerRuns {
    by_agent: StdMutex<BTreeMap<String, String>>,
}
impl SettledWorkerRuns {
    pub(super) fn record(&self, agent_id: &str, leased_agent_id: &str, worker_run_id: &str) {
        self.by_agent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                agent_id.into(),
                crate::provider::projected_leased_provider_run_id(leased_agent_id, worker_run_id),
            );
    }
    /// The projected id of the run this agent's lease last settled a turn on.
    pub(super) fn last(&self, agent_id: &str, leased_agent_id: &str) -> Option<String> {
        self.by_agent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(agent_id)
            .filter(|run| {
                crate::provider::worker_provider_run_id_from_projected_leased_id(
                    leased_agent_id,
                    run,
                )
                .is_some()
            })
            .cloned()
    }
}
