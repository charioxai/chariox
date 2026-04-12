use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::DaemonError;

use super::{
    opencode_runtime::OpenCodeRuntimeState, CodexRuntimeState, ProviderPromptAbortCompletion,
    ProviderPromptAbortJob, ProviderPromptSubmitCompletion, ProviderPromptSubmitJob,
};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    operation_lanes: ProviderRunOperationLanes,
    workers: Arc<Mutex<BTreeMap<String, mpsc::Sender<ProviderRunActorCommand>>>>,
    codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    structured_prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderRunOperationLanes {
    lanes: Arc<Mutex<BTreeMap<String, Arc<Semaphore>>>>,
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
    Stop,
}

impl ProviderRunOperationLanes {
    pub(crate) async fn acquire(&self, provider_run_id: &str) -> OwnedSemaphorePermit {
        let semaphore = {
            let mut lanes = self
                .lanes
                .lock()
                .expect("provider run operation lane map poisoned");
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

    fn forget(&self, provider_run_id: &str) {
        let mut lanes = self
            .lanes
            .lock()
            .expect("provider run operation lane map poisoned");
        lanes.remove(provider_run_id);
    }
}

impl ProviderRunActorMailbox {
    pub(crate) fn operation_lanes(&self) -> ProviderRunOperationLanes {
        self.operation_lanes.clone()
    }

    pub(crate) fn insert_codex_runtime(&self, run_id: String, state: CodexRuntimeState) {
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .insert(run_id, state);
    }

    pub(crate) fn take_codex_runtime(&self, run_id: &str) -> Option<CodexRuntimeState> {
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .remove(run_id)
    }

    pub(crate) fn codex_runtime_exists(&self, run_id: &str) -> bool {
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .contains_key(run_id)
    }

    pub(crate) fn with_codex_runtime_mut<R>(
        &self,
        run_id: &str,
        f: impl FnOnce(&mut CodexRuntimeState) -> R,
    ) -> Option<R> {
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .get_mut(run_id)
            .map(f)
    }

    pub(crate) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .insert(run_id, state);
    }

    pub(crate) fn take_opencode_runtime(&self, run_id: &str) -> Option<OpenCodeRuntimeState> {
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .remove(run_id)
    }

    pub(crate) fn with_opencode_runtime<R>(
        &self,
        run_id: &str,
        f: impl FnOnce(&OpenCodeRuntimeState) -> R,
    ) -> Option<R> {
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .get(run_id)
            .map(f)
    }

    pub(crate) fn with_opencode_runtime_mut<R>(
        &self,
        run_id: &str,
        f: impl FnOnce(&mut OpenCodeRuntimeState) -> R,
    ) -> Option<R> {
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .get_mut(run_id)
            .map(f)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, run_id: &str) -> bool {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .contains(run_id)
    }

    pub(crate) fn mark_structured_prompt_io_in_flight(&self, run_id: String) {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .insert(run_id);
    }

    pub(crate) fn clear_structured_prompt_io_in_flight(&self, run_id: &str) {
        self.structured_prompt_submissions
            .lock()
            .expect("structured prompt submission set poisoned")
            .remove(run_id);
    }

    pub(crate) fn clear_runtime(&self, run_id: &str) {
        self.clear_structured_prompt_io_in_flight(run_id);
        self.codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .remove(run_id);
        if let Some(state) = self
            .opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .remove(run_id)
        {
            state.stop();
        }
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

    pub(crate) fn stop_run(&self, provider_run_id: &str) {
        self.operation_lanes.forget(provider_run_id);
        let sender = {
            let mut workers = self
                .workers
                .lock()
                .expect("provider run actor worker map poisoned");
            workers.remove(provider_run_id)
        };
        if let Some(sender) = sender {
            let _ = sender.send(ProviderRunActorCommand::Stop);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_run_removes_worker_and_lane_registration() {
        let mailbox = ProviderRunActorMailbox::default();
        let _sender = mailbox.worker_for_run("run-1");
        let _permit = mailbox.operation_lanes.acquire("run-1").await;
        mailbox.mark_structured_prompt_io_in_flight("run-1".to_string());
        assert_eq!(
            mailbox
                .workers
                .lock()
                .expect("worker map should not be poisoned")
                .len(),
            1
        );
        assert_eq!(
            mailbox
                .operation_lanes
                .lanes
                .lock()
                .expect("lane map should not be poisoned")
                .len(),
            1
        );
        assert!(mailbox.structured_prompt_io_in_flight("run-1"));

        mailbox.clear_runtime("run-1");
        mailbox.stop_run("run-1");

        assert_eq!(
            mailbox
                .workers
                .lock()
                .expect("worker map should not be poisoned")
                .len(),
            0
        );
        assert_eq!(
            mailbox
                .operation_lanes
                .lanes
                .lock()
                .expect("lane map should not be poisoned")
                .len(),
            0
        );
        assert!(!mailbox.structured_prompt_io_in_flight("run-1"));
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
