use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};

use crate::error::DaemonError;

use super::{
    ProviderPromptAbortCompletion, ProviderPromptAbortJob, ProviderPromptSubmitCompletion,
    ProviderPromptSubmitJob,
};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    operation_lanes: ProviderRunOperationLanes,
    workers: Arc<Mutex<BTreeMap<String, mpsc::Sender<ProviderRunActorCommand>>>>,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderRunOperationLanes {
    lanes: Arc<AsyncMutex<BTreeMap<String, Arc<Semaphore>>>>,
}

pub(crate) struct FinishedProviderPromptSubmitJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) completion: ProviderPromptSubmitCompletion,
    pub(crate) result: Result<(), DaemonError>,
}

pub(crate) struct FinishedProviderPromptAbortJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) completion: ProviderPromptAbortCompletion,
    pub(crate) result: Result<(), DaemonError>,
}

enum ProviderRunActorCommand {
    Submit {
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        job: ProviderPromptSubmitJob,
    },
    Abort {
        session_id: String,
        provider_run_id: String,
        job: ProviderPromptAbortJob,
    },
}

impl ProviderRunOperationLanes {
    pub(crate) async fn acquire(&self, provider_run_id: &str) -> OwnedSemaphorePermit {
        let semaphore = {
            let mut lanes = self.lanes.lock().await;
            Arc::clone(
                lanes
                    .entry(provider_run_id.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(1))),
            )
        };
        semaphore
            .acquire_owned()
            .await
            .expect("provider run operation lane semaphore closed")
    }
}

impl ProviderRunActorMailbox {
    pub(crate) fn operation_lanes(&self) -> ProviderRunOperationLanes {
        self.operation_lanes.clone()
    }

    pub(crate) fn spawn_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        job: ProviderPromptSubmitJob,
    ) {
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::Submit {
            session_id,
            provider_run_id: provider_run_id.clone(),
            agent_id,
            job,
        }) {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "structured prompt submit command enqueue failed",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(crate) fn spawn_abort(
        &self,
        session_id: String,
        provider_run_id: String,
        job: ProviderPromptAbortJob,
    ) {
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::Abort {
            session_id,
            provider_run_id: provider_run_id.clone(),
            job,
        }) {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "structured prompt abort command enqueue failed",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(crate) fn drain_finished_submits(&self) -> Vec<FinishedProviderPromptSubmitJob> {
        match self.finished_submits.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt submit completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn drain_finished_aborts(&self) -> Vec<FinishedProviderPromptAbortJob> {
        match self.finished_aborts.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "structured prompt abort completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    fn worker_for_run(&self, provider_run_id: &str) -> mpsc::Sender<ProviderRunActorCommand> {
        let mut workers = self
            .workers
            .lock()
            .expect("provider run actor worker map poisoned");
        workers
            .entry(provider_run_id.to_string())
            .or_insert_with(|| {
                Self::spawn_worker(
                    provider_run_id.to_string(),
                    Arc::clone(&self.finished_submits),
                    Arc::clone(&self.finished_aborts),
                )
            })
            .clone()
    }

    fn spawn_worker(
        provider_run_id: String,
        finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
        finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    ) -> mpsc::Sender<ProviderRunActorCommand> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    ProviderRunActorCommand::Submit {
                        session_id,
                        provider_run_id,
                        agent_id,
                        job,
                    } => {
                        let (completion, result) = job.execute();
                        let finished = FinishedProviderPromptSubmitJob {
                            session_id,
                            provider_run_id,
                            agent_id,
                            completion,
                            result,
                        };
                        push_finished_submit(&finished_submits, finished);
                    }
                    ProviderRunActorCommand::Abort {
                        session_id,
                        provider_run_id,
                        job,
                    } => {
                        let (completion, result) = job.execute();
                        let finished = FinishedProviderPromptAbortJob {
                            session_id,
                            provider_run_id,
                            completion,
                            result,
                        };
                        push_finished_abort(&finished_aborts, finished);
                    }
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

fn push_finished_submit(
    finished_submits: &Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished: FinishedProviderPromptSubmitJob,
) {
    match finished_submits.lock() {
        Ok(mut jobs) => jobs.push(finished),
        Err(error) => {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "structured prompt submit completion queue poisoned",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }
}

fn push_finished_abort(
    finished_aborts: &Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    finished: FinishedProviderPromptAbortJob,
) {
    match finished_aborts.lock() {
        Ok(mut jobs) => jobs.push(finished),
        Err(error) => {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "structured prompt abort completion queue poisoned",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }
}
