use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod command_enqueue;
mod command_execution;
mod finished_jobs;
mod in_flight;
mod native_interaction;
mod operation_lanes;
mod runtime_slots;
mod worker;
use finished_jobs::{
    drain_finished_aborts, drain_finished_output_polls, drain_finished_selection_syncs,
    drain_finished_submits, push_finished_output_poll,
};
pub(crate) use finished_jobs::{
    FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob, FinishedProviderPromptSubmitJob,
    FinishedProviderRunSelectionSyncJob,
};
use in_flight::ProviderRunInFlightState;
pub(crate) use native_interaction::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionBridgeStore,
    ProviderNativeInteractionResolution,
};
pub(crate) use operation_lanes::ProviderRunOperationLanes;
use runtime_slots::ProviderRunRuntimeRegistry;
use worker::{ProviderRunActorCommand, ProviderRunWorkerDeps};

use crate::error::DaemonError;
use crate::session::PromptAttachment;

use super::{
    opencode_runtime::OpenCodeRuntimeState, ClaudeRuntimeState, CodexRuntimeState,
    RuntimeProviderRun,
};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    operation_lanes: ProviderRunOperationLanes,
    native_interaction_bridge: ProviderNativeInteractionBridgeStore,
    workers: Arc<Mutex<BTreeMap<String, mpsc::SyncSender<ProviderRunActorCommand>>>>,
    runtime_registry: ProviderRunRuntimeRegistry,
    in_flight: ProviderRunInFlightState,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
}

impl ProviderRunActorMailbox {
    pub(crate) fn operation_lanes(&self) -> ProviderRunOperationLanes {
        self.operation_lanes.clone()
    }

    pub(crate) fn set_native_interaction_bridge(
        &self,
        bridge: Arc<dyn ProviderNativeInteractionBridge>,
    ) {
        self.native_interaction_bridge.set(bridge);
    }

    pub(crate) fn insert_claude_runtime(&self, run_id: String, state: ClaudeRuntimeState) {
        self.runtime_registry.insert_claude_runtime(run_id, state);
    }

    pub(crate) fn insert_codex_runtime(&self, run_id: String, state: CodexRuntimeState) {
        self.runtime_registry.insert_codex_runtime(run_id, state);
    }

    pub(crate) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        self.runtime_registry.insert_opencode_runtime(run_id, state);
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
        self.clear_structured_prompt_io_in_flight(run_id);
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

    fn structured_output_poll_in_flight(&self, run_id: &str) -> bool {
        self.in_flight.output_poll_in_flight(run_id)
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
