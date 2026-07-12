use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc as tokio_mpsc, OwnedSemaphorePermit, Semaphore};

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::RuntimeProviderRun;

use super::command_execution::{
    execute_abort_command, execute_output_poll_command, execute_selection_sync_command,
    execute_submit_command, execute_terminate_command, execute_utility_command,
};
use super::finished_jobs::{
    push_finished_abort, push_finished_output_poll, push_finished_selection_sync,
    push_finished_submit, FinishedProviderOutputPollJob, FinishedProviderPromptAbortJob,
    FinishedProviderPromptSubmitJob, FinishedProviderRunSelectionSyncJob,
};
use super::in_flight::ProviderRunInFlightState;
use super::native_interaction::ProviderNativeInteractionBridgeStore;
use super::runtime_slots::ProviderRunRuntimeRegistry;
use super::ProviderRunActorCompletionSignal;

const PROVIDER_RUN_COMMAND_QUEUE_LIMIT: usize = 64;

fn provider_actor_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("arroba-provider-actor")
            .enable_all()
            .build()
            .expect("provider actor runtime should start")
    })
}

pub(super) enum ProviderRunActorCommand {
    Submit {
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        envelope: PromptEnvelope,
    },
    Utility {
        provider_run_id: String,
        run: RuntimeProviderRun,
        envelope: PromptEnvelope,
        timeout: Duration,
        response: mpsc::Sender<Result<String, DaemonError>>,
    },
    Abort {
        session_id: String,
        provider_run_id: String,
        run: RuntimeProviderRun,
    },
    Terminate {
        provider_run_id: String,
        run: RuntimeProviderRun,
        completion: mpsc::Sender<()>,
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

#[derive(Clone)]
pub(super) struct ProviderRunWorkerDeps {
    pub(super) native_interaction_bridge: ProviderNativeInteractionBridgeStore,
    pub(super) runtime_registry: ProviderRunRuntimeRegistry,
    pub(super) in_flight: ProviderRunInFlightState,
    pub(super) finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    pub(super) finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    pub(super) finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    pub(super) finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    pub(super) completion_signal: ProviderRunActorCompletionSignal,
    pub(super) output_poll_delays: Arc<Mutex<BTreeMap<String, Duration>>>,
    pub(super) blocking_executor_permits: Arc<Semaphore>,
}

impl ProviderRunWorkerDeps {
    pub(super) fn spawn(
        self,
        provider_run_id: String,
    ) -> tokio_mpsc::Sender<ProviderRunActorCommand> {
        let (tx, mut rx) = tokio_mpsc::channel(PROVIDER_RUN_COMMAND_QUEUE_LIMIT);
        provider_actor_runtime().spawn(async move {
            while let Some(command) = rx.recv().await {
                let Ok(permit) = Arc::clone(&self.blocking_executor_permits)
                    .acquire_owned()
                    .await
                else {
                    break;
                };
                let worker = self.clone();
                match tokio::task::spawn_blocking(move || worker.execute(command, permit)).await {
                    Ok(true) | Err(_) => break,
                    Ok(false) => {}
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

    fn execute(&self, command: ProviderRunActorCommand, _permit: OwnedSemaphorePermit) -> bool {
        match command {
            ProviderRunActorCommand::Submit {
                session_id,
                provider_run_id,
                agent_id,
                run,
                envelope,
            } => {
                let result = execute_submit_command(&self.runtime_registry, run, envelope);
                self.in_flight.clear_prompt_io_in_flight(&provider_run_id);
                let finished = FinishedProviderPromptSubmitJob {
                    session_id,
                    provider_run_id: provider_run_id.clone(),
                    agent_id,
                    result,
                };
                push_finished_submit(&self.finished_submits, finished);
                self.completion_signal.record_completion(&provider_run_id);
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
                    provider_run_id: provider_run_id.clone(),
                    result,
                };
                push_finished_abort(&self.finished_aborts, finished);
                self.completion_signal.record_completion(&provider_run_id);
            }
            ProviderRunActorCommand::Utility {
                provider_run_id,
                run,
                envelope,
                timeout,
                response,
            } => {
                let result =
                    execute_utility_command(&self.runtime_registry, run, envelope, timeout);
                let _ = response.send(result);
                self.in_flight.clear_prompt_io_in_flight(&provider_run_id);
            }
            ProviderRunActorCommand::Terminate {
                provider_run_id,
                run,
                completion,
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
                let _ = completion.send(());
                return true;
            }
            ProviderRunActorCommand::SyncSelection {
                provider_run_id,
                run,
            } => {
                let result =
                    execute_selection_sync_command(&self.runtime_registry, &provider_run_id, &run);
                let finished = FinishedProviderRunSelectionSyncJob {
                    provider_run_id: provider_run_id.clone(),
                    result,
                };
                push_finished_selection_sync(&self.finished_selection_syncs, finished);
                self.completion_signal.record_completion(&provider_run_id);
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
                    provider_run_id: provider_run_id.clone(),
                    result,
                };
                push_finished_output_poll(&self.finished_output_polls, finished);
                self.completion_signal.record_completion(&provider_run_id);
            }
            ProviderRunActorCommand::Stop => return true,
        }
        false
    }
}
