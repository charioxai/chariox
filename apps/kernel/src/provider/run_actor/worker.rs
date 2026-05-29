use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::provider::RuntimeProviderRun;
use crate::prompt_assembly::PromptEnvelope;

use super::command_execution::{
    execute_abort_command, execute_output_poll_command, execute_selection_sync_command,
    execute_submit_command, execute_terminate_command,
};
use super::finished_jobs::{
    push_finished_abort, push_finished_output_poll, push_finished_selection_sync,
    push_finished_submit, FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob,
    FinishedProviderPromptSubmitJob, FinishedProviderRunSelectionSyncJob,
};
use super::in_flight::ProviderRunInFlightState;
use super::native_interaction::ProviderNativeInteractionBridgeStore;
use super::runtime_slots::ProviderRunRuntimeRegistry;

const PROVIDER_RUN_COMMAND_QUEUE_LIMIT: usize = 64;

pub(super) enum ProviderRunActorCommand {
    Submit {
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        envelope: PromptEnvelope,
    },
    Abort {
        session_id: String,
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    Terminate {
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    SyncSelection {
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    PollOutput {
        provider_run_id: String,
        run: RuntimeProviderRun,
        output_poll_delay: Duration,
    },
    Stop,
}

pub(super) struct ProviderRunWorkerDeps {
    pub(super) native_interaction_bridge: ProviderNativeInteractionBridgeStore,
    pub(super) runtime_registry: ProviderRunRuntimeRegistry,
    pub(super) in_flight: ProviderRunInFlightState,
    pub(super) finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    pub(super) finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    pub(super) finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    pub(super) finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    pub(super) output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
}

impl ProviderRunWorkerDeps {
    pub(super) fn spawn(
        self,
        provider_run_id: String,
    ) -> mpsc::SyncSender<ProviderRunActorCommand> {
        let (tx, rx) = mpsc::sync_channel(PROVIDER_RUN_COMMAND_QUEUE_LIMIT);
        thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    ProviderRunActorCommand::Submit {
                        session_id,
                        provider_run_id,
                        agent_id,
                        run,
                        envelope,
                    } => {
                        let result =
                            execute_submit_command(&self.runtime_registry, run, envelope);
                        self.in_flight.clear_prompt_io_in_flight(&provider_run_id);
                        let finished = FinishedProviderPromptSubmitJob {
                            session_id,
                            provider_run_id,
                            agent_id,
                            result,
                        };
                        push_finished_submit(&self.finished_submits, finished);
                    }
                    ProviderRunActorCommand::Abort {
                        session_id,
                        provider_run_id,
                        run,
                    } => {
                        let result = execute_abort_command(&self.runtime_registry, run);
                        self.in_flight.clear_prompt_io_in_flight(&provider_run_id);
                        let finished = FinishedProviderPromptAbortJob {
                            session_id,
                            provider_run_id,
                            result,
                        };
                        push_finished_abort(&self.finished_aborts, finished);
                    }
                    ProviderRunActorCommand::Terminate {
                        provider_run_id,
                        run,
                    } => {
                        if let Err(error) = execute_terminate_command(&self.runtime_registry, run) {
                            crate::logging::error_with_fields(
                                "daemon.provider_run_actor",
                                "structured provider termination abort failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        self.in_flight.clear_prompt_io_in_flight(&provider_run_id);
                        self.output_poll_delays
                            .lock()
                            .expect("provider output poll delay map poisoned")
                            .remove(&provider_run_id);
                        self.runtime_registry
                            .clear_runtime_state(&provider_run_id, true);
                        break;
                    }
                    ProviderRunActorCommand::SyncSelection {
                        provider_run_id,
                        run,
                    } => {
                        let result = execute_selection_sync_command(
                            &self.runtime_registry,
                            &provider_run_id,
                            &run,
                        );
                        let finished = FinishedProviderRunSelectionSyncJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_selection_sync(&self.finished_selection_syncs, finished);
                    }
                    ProviderRunActorCommand::PollOutput {
                        provider_run_id,
                        run,
                        output_poll_delay,
                    } => {
                        let result = execute_output_poll_command(
                            &self.native_interaction_bridge,
                            &self.runtime_registry,
                            &run,
                            output_poll_delay,
                        );
                        self.in_flight.clear_output_poll_in_flight(&provider_run_id);
                        let finished = FinishedProviderOutputPollJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_output_poll(&self.finished_output_polls, finished);
                    }
                    ProviderRunActorCommand::Stop => break,
                }
            }
            crate::logging::info_with_fields(
                "daemon.provider_run_actor",
                "provider run actor worker stopped",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                }),
            );
        });
        tx
    }
}
