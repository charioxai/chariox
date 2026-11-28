mod utility_cancellation;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, Semaphore, mpsc as tokio_mpsc};

pub(in crate::provider) mod assessment;
mod command_enqueue;
mod command_execution;
mod finished_jobs;
mod in_flight;
mod native_interaction;
mod operation_lanes;
#[cfg(test)]
mod refusal_tests;
mod runtime_slots;
mod worker;
pub(crate) use command_execution::run_room_commands_on_fresh_claude;
use finished_jobs::push_finished_output_poll;
pub(crate) use finished_jobs::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    FinishedProviderRunSelectionSyncJob, OutputPublicationClaim, ProviderOutputPromptIdentity,
    ProviderPromptSubmitAcknowledgement, ProviderPromptSubmitReceipt,
};
use finished_jobs::{
    drain_finished_aborts, drain_finished_output_polls, drain_finished_selection_syncs,
    drain_finished_submits, push_finished_submit,
};
use in_flight::ProviderRunInFlightState;
pub(crate) use native_interaction::{
    CodexApprovalHandover, CodexApprovalOutcome, NativeApprovalGoal,
    ProviderNativeInteractionBridge, ProviderNativeInteractionBridgeStore,
    ProviderNativeInteractionResolution, setup_turn_origin,
};
pub(crate) use operation_lanes::{ProviderRunOperationLanes, ProviderRunOperationPermit};
use runtime_slots::ProviderRunRuntimeRegistry;
use worker::{ProviderRunActorCommand, ProviderRunWorkerDeps};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;

use super::{
    ClaudeRuntimeState, CodexRuntimeState, ProviderUtilityExecutionPolicy, RuntimeProviderRun,
    opencode_runtime::OpenCodeRuntimeState,
};

#[derive(Clone)]
pub(crate) struct ProviderRunActorMailbox {
    pub(crate) submit_epoch: u64,
    operation_lanes: ProviderRunOperationLanes,
    native_interaction_bridge: ProviderNativeInteractionBridgeStore,
    workers: Arc<Mutex<BTreeMap<String, tokio_mpsc::Sender<ProviderRunActorCommand>>>>,
    runtime_registry: ProviderRunRuntimeRegistry,
    in_flight: ProviderRunInFlightState,
    utility_cancellations: utility_cancellation::MetadataUtilityCancellations,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    output_publications: finished_jobs::OutputPublications,
    completion_signal: ProviderRunActorCompletionSignal,
    output_ready_signal: super::output_ready::ProviderOutputReadySignal,
    output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
    #[cfg(test)]
    manual_output_polls: Arc<std::sync::atomic::AtomicBool>,
    /// Test seam: a terminate waits for its actor's acknowledgement as long as production does.
    #[cfg(test)]
    production_terminate_wait: Arc<std::sync::atomic::AtomicBool>,
    blocking_executor_permits: Arc<Semaphore>,
}

impl Default for ProviderRunActorMailbox {
    fn default() -> Self {
        Self {
            submit_epoch: rand::random(),
            operation_lanes: ProviderRunOperationLanes::default(),
            native_interaction_bridge: ProviderNativeInteractionBridgeStore::default(),
            workers: Arc::new(Mutex::new(BTreeMap::new())),
            runtime_registry: ProviderRunRuntimeRegistry::default(),
            in_flight: ProviderRunInFlightState::default(),
            utility_cancellations: utility_cancellation::MetadataUtilityCancellations::default(),
            finished_submits: Arc::new(Mutex::new(Vec::new())),
            finished_aborts: Arc::new(Mutex::new(Vec::new())),
            finished_selection_syncs: Arc::new(Mutex::new(Vec::new())),
            finished_output_polls: Arc::new(Mutex::new(Vec::new())),
            output_publications: Default::default(),
            completion_signal: ProviderRunActorCompletionSignal::default(),
            output_ready_signal: Default::default(),
            output_poll_delays: Arc::new(Mutex::new(BTreeMap::new())),
            #[cfg(test)]
            manual_output_polls: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(test)]
            production_terminate_wait: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            blocking_executor_permits: Arc::new(Semaphore::new(64)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderRunActorCompletionSignal {
    inner: Arc<ProviderRunActorCompletionSignalState>,
}

#[derive(Debug, Default)]
struct ProviderRunActorCompletionSignalState {
    sequence: AtomicU64,
    notify: Notify,
    ready_provider_runs: Mutex<BTreeSet<String>>,
}

impl ProviderRunActorMailbox {
    /// A test that needs the closing output of a terminated run waits for the actor as production
    /// does; the short default only keeps the tests of an actor that never answers quick.
    #[cfg(test)]
    pub(crate) fn wait_for_terminate_like_production_in_tests(&self) {
        self.production_terminate_wait
            .store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn use_manual_output_polls_for_tests(&self) {
        self.manual_output_polls.store(true, Ordering::Relaxed);
    }

    pub(crate) fn operation_lanes(&self) -> ProviderRunOperationLanes {
        self.operation_lanes.clone()
    }

    pub(crate) fn output_ready_signal(&self) -> super::output_ready::ProviderOutputReadySignal {
        self.output_ready_signal.clone()
    }

    pub(crate) fn output_publication_pending(&self, run_id: &str) -> bool {
        self.output_publications.pending(run_id)
    }

    pub(crate) fn completion_signal(&self) -> ProviderRunActorCompletionSignal {
        self.completion_signal.clone()
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.native_interaction_bridge.set(bridge);
    }

    pub(crate) fn native_interaction_bridge(
        &self,
    ) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
        self.native_interaction_bridge.read()
    }

    pub(crate) fn claude_process_identities(
        &self,
    ) -> Vec<(
        String,
        crate::runtime::kernel_access::process::ProcessIdentity,
    )> {
        self.runtime_registry.claude_process_identities()
    }

    /// Runs a command holds: their turns are not at a safe point for a hot reload.
    pub(crate) fn busy_runs(&self) -> Vec<String> {
        let mut runs = self.runtime_registry.busy_runs();
        // A prompt or steer queued behind the worker is work too: it takes the runtime next.
        runs.extend(self.in_flight.prompt_io_in_flight_runs());
        runs.extend(self.in_flight.output_poll_in_flight_runs());
        runs.extend(self.in_flight.plan_publication_in_flight_runs());
        runs.sort();
        runs.dedup();
        runs
    }

    pub(crate) fn claude_runs_exited_while_idle(&self, only: Option<&str>) -> Vec<(String, bool)> {
        self.runtime_registry.claude_runs_exited_while_idle(only)
    }

    pub(crate) fn insert_claude_runtime(&self, run_id: String, state: ClaudeRuntimeState) {
        state
            .output_notifier
            .bind(run_id.clone(), self.output_ready_signal.clone());
        self.runtime_registry.insert_claude_runtime(run_id, state);
    }

    #[cfg(test)]
    pub(crate) fn claude_owned_process(
        &self,
        run_id: &str,
    ) -> Option<crate::provider::claude_runtime::ClaudeRuntimeProcess> {
        self.runtime_registry.claude_owned_process(run_id)
    }

    pub(crate) fn claude_owned_processes(
        &self,
        run_id: &str,
    ) -> Vec<crate::provider::claude_runtime::ClaudeRuntimeProcess> {
        self.runtime_registry.claude_owned_processes(run_id)
    }

    pub(crate) fn insert_codex_runtime(&self, run_id: String, mut state: CodexRuntimeState) {
        state.bind_output_ready(run_id.clone(), self.output_ready_signal.clone());
        self.runtime_registry.insert_codex_runtime(run_id, state);
    }

    pub(crate) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        state.bind_output_ready(run_id.clone(), self.output_ready_signal.clone());
        self.runtime_registry.insert_opencode_runtime(run_id, state);
    }

    /// Whether any run still has an output poll queued or running: its result
    /// is not yet there to apply.
    pub(crate) fn any_output_poll_in_flight(&self) -> bool {
        self.in_flight.any_output_poll_in_flight()
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, run_id: &str) -> bool {
        self.in_flight.prompt_io_in_flight(run_id)
    }

    pub(crate) fn structured_runtime_state_bound(&self, run_id: &str) -> bool {
        self.runtime_registry.state_bound(run_id)
    }

    pub(crate) fn mark_structured_prompt_io_in_flight(&self, run_id: String) {
        self.in_flight.mark_prompt_io_in_flight(run_id);
    }

    pub(crate) fn clear_structured_prompt_io_in_flight(&self, run_id: &str) {
        self.in_flight.clear_prompt_io_in_flight(run_id);
    }

    pub(crate) fn clear_runtime(&self, run_id: &str) {
        self.output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned")
            .remove(run_id);
        self.in_flight.reset_prompt_io_in_flight(run_id);
        self.clear_structured_output_poll_in_flight(run_id);
        self.runtime_registry.clear_runtime(run_id, true);
    }

    #[doc(hidden)]
    pub(crate) fn set_output_poll_delay_for_tests(&self, run_id: &str, delay: Duration) {
        let mut delays = self
            .output_poll_delays
            .lock()
            .expect("provider output poll delay map poisoned");
        if delay.is_zero() {
            delays.remove(run_id);
        } else {
            delays.insert(run_id.to_string(), delay);
        }
    }

    fn mark_structured_output_poll_in_flight(&self, run_id: String) -> bool {
        self.in_flight.mark_output_poll_in_flight(run_id)
    }

    fn clear_structured_output_poll_in_flight(&self, run_id: &str) {
        self.in_flight.clear_output_poll_in_flight(run_id);
    }

    #[cfg(test)]
    fn structured_output_poll_in_flight(&self, run_id: &str) -> bool {
        self.in_flight.output_poll_in_flight(run_id)
    }
}

impl ProviderRunActorCompletionSignal {
    pub(crate) fn sequence(&self) -> u64 {
        self.inner.sequence.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.inner.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }

    pub(crate) fn take_ready_provider_run_ids(&self) -> BTreeSet<String> {
        let mut ready = self
            .inner
            .ready_provider_runs
            .lock()
            .expect("provider run completion ready set poisoned");
        std::mem::take(&mut *ready)
    }

    pub(super) fn record_completion(&self, provider_run_id: &str) {
        self.inner
            .ready_provider_runs
            .lock()
            .expect("provider run completion ready set poisoned")
            .insert(provider_run_id.to_string());
        self.inner.sequence.fetch_add(1, Ordering::AcqRel);
        self.inner.notify.notify_waiters();
    }
}

fn provider_actor_enqueue_error(
    operation: &'static str,
    provider_run_id: &str,
    error_message: String,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: format!(
            "provider run actor queue rejected command for `{provider_run_id}`: {error_message}"
        ),
    }
}
