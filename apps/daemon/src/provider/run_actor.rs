use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::DaemonError;
use crate::session::PromptAttachment;

use super::{
    codex_runtime::{abort_codex_turn, drain_codex_events, submit_codex_prompt},
    opencode_binding::{
        abort_opencode_session, submit_opencode_prompt, sync_opencode_run_selection_for_session,
        OpenCodeRunSelection,
    },
    opencode_runtime::{drain_opencode_events, OpenCodeRuntimeState},
    CodexRuntimeState, ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
    RuntimeProviderRun,
};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    operation_lanes: ProviderRunOperationLanes,
    workers: Arc<Mutex<BTreeMap<String, mpsc::Sender<ProviderRunActorCommand>>>>,
    codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    structured_prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
    structured_output_polls: Arc<Mutex<BTreeSet<String>>>,
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
    finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderRunOperationLanes {
    lanes: Arc<Mutex<BTreeMap<String, Arc<Semaphore>>>>,
}

pub(crate) struct FinishedProviderPromptSubmitJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) result: Result<(), DaemonError>,
}

pub(crate) struct FinishedProviderPromptAbortJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) result: Result<(), DaemonError>,
}

pub(crate) struct FinishedProviderRunSelectionSyncJob {
    pub(crate) provider_run_id: String,
    pub(crate) result: Result<OpenCodeRunSelection, DaemonError>,
}

pub(crate) struct FinishedProviderOutputPollJob {
    pub(crate) provider_run_id: String,
    pub(crate) result: Result<Option<ProviderPromptSignalBatch>, DaemonError>,
}

enum ProviderRunActorCommand {
    Submit {
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        prompt: String,
        attachments: Vec<PromptAttachment>,
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
    },
    PollOutput {
        provider_run_id: String,
        run: RuntimeProviderRun,
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

    pub(crate) fn insert_opencode_runtime(&self, run_id: String, state: OpenCodeRuntimeState) {
        self.opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .insert(run_id, state);
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
        self.clear_structured_output_poll_in_flight(run_id);
        clear_runtime_state(&self.codex_runs, &self.opencode_runs, run_id);
    }

    fn mark_structured_output_poll_in_flight(&self, run_id: String) -> bool {
        self.structured_output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .insert(run_id)
    }

    fn clear_structured_output_poll_in_flight(&self, run_id: &str) {
        self.structured_output_polls
            .lock()
            .expect("structured output poll set poisoned")
            .remove(run_id);
    }

    pub(crate) fn spawn_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: RuntimeProviderRun,
        prompt: String,
        attachments: Vec<PromptAttachment>,
    ) {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::Submit {
            session_id,
            provider_run_id: provider_run_id.clone(),
            agent_id,
            run,
            prompt,
            attachments,
        }) {
            self.clear_structured_prompt_io_in_flight(&provider_run_id);
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
        run: RuntimeProviderRun,
    ) {
        self.mark_structured_prompt_io_in_flight(provider_run_id.clone());
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::Abort {
            session_id,
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            self.clear_structured_prompt_io_in_flight(&provider_run_id);
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

    pub(crate) fn spawn_terminate(&self, provider_run_id: String, run: RuntimeProviderRun) {
        self.operation_lanes.forget(&provider_run_id);
        let sender = {
            let mut workers = self
                .workers
                .lock()
                .expect("provider run actor worker map poisoned");
            workers.remove(&provider_run_id).unwrap_or_else(|| {
                Self::spawn_worker(
                    provider_run_id.clone(),
                    Arc::clone(&self.codex_runs),
                    Arc::clone(&self.opencode_runs),
                    Arc::clone(&self.structured_prompt_submissions),
                    Arc::clone(&self.structured_output_polls),
                    Arc::clone(&self.finished_submits),
                    Arc::clone(&self.finished_aborts),
                    Arc::clone(&self.finished_selection_syncs),
                    Arc::clone(&self.finished_output_polls),
                )
            })
        };
        if let Err(error) = sender.send(ProviderRunActorCommand::Terminate {
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            self.clear_structured_prompt_io_in_flight(&provider_run_id);
            self.clear_runtime(&provider_run_id);
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "provider run actor terminate command enqueue failed",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(crate) fn spawn_selection_sync(&self, provider_run_id: String) {
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::SyncSelection {
            provider_run_id: provider_run_id.clone(),
        }) {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "provider run selection sync command enqueue failed",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(crate) fn spawn_output_poll(
        &self,
        provider_run_id: String,
        run: RuntimeProviderRun,
    ) -> bool {
        if !self.mark_structured_output_poll_in_flight(provider_run_id.clone()) {
            return false;
        }
        let sender = self.worker_for_run(&provider_run_id);
        if let Err(error) = sender.send(ProviderRunActorCommand::PollOutput {
            provider_run_id: provider_run_id.clone(),
            run,
        }) {
            self.clear_structured_output_poll_in_flight(&provider_run_id);
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "provider run output poll command enqueue failed",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
            return false;
        }
        true
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

    pub(crate) fn drain_finished_selection_syncs(
        &self,
    ) -> Vec<FinishedProviderRunSelectionSyncJob> {
        match self.finished_selection_syncs.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run selection sync completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn drain_finished_output_polls(&self) -> Vec<FinishedProviderOutputPollJob> {
        match self.finished_output_polls.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run output poll completion queue poisoned",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn return_finished_output_polls(&self, jobs: Vec<FinishedProviderOutputPollJob>) {
        if jobs.is_empty() {
            return;
        }
        match self.finished_output_polls.lock() {
            Ok(mut existing_jobs) => existing_jobs.extend(jobs),
            Err(error) => {
                crate::logging::error_with_fields(
                    "daemon.provider_run_actor",
                    "provider run output poll completion queue poisoned while restoring jobs",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
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
                    Arc::clone(&self.codex_runs),
                    Arc::clone(&self.opencode_runs),
                    Arc::clone(&self.structured_prompt_submissions),
                    Arc::clone(&self.structured_output_polls),
                    Arc::clone(&self.finished_submits),
                    Arc::clone(&self.finished_aborts),
                    Arc::clone(&self.finished_selection_syncs),
                    Arc::clone(&self.finished_output_polls),
                )
            })
            .clone()
    }

    fn spawn_worker(
        provider_run_id: String,
        codex_runs: Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
        opencode_runs: Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
        structured_prompt_submissions: Arc<Mutex<BTreeSet<String>>>,
        structured_output_polls: Arc<Mutex<BTreeSet<String>>>,
        finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
        finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
        finished_selection_syncs: Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
        finished_output_polls: Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    ) -> mpsc::Sender<ProviderRunActorCommand> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    ProviderRunActorCommand::Submit {
                        session_id,
                        provider_run_id,
                        agent_id,
                        run,
                        prompt,
                        attachments,
                    } => {
                        let result = execute_submit_command(
                            &codex_runs,
                            &opencode_runs,
                            run,
                            prompt,
                            attachments,
                        );
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderPromptSubmitJob {
                            session_id,
                            provider_run_id,
                            agent_id,
                            result,
                        };
                        push_finished_submit(&finished_submits, finished);
                    }
                    ProviderRunActorCommand::Abort {
                        session_id,
                        provider_run_id,
                        run,
                    } => {
                        let result = execute_abort_command(&codex_runs, &opencode_runs, run);
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderPromptAbortJob {
                            session_id,
                            provider_run_id,
                            result,
                        };
                        push_finished_abort(&finished_aborts, finished);
                    }
                    ProviderRunActorCommand::Terminate {
                        provider_run_id,
                        run,
                    } => {
                        if let Err(error) =
                            execute_terminate_command(&codex_runs, &opencode_runs, run)
                        {
                            crate::logging::error_with_fields(
                                "daemon.provider_run_actor",
                                "structured provider termination abort failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        clear_structured_prompt_io_in_flight(
                            &structured_prompt_submissions,
                            &provider_run_id,
                        );
                        clear_runtime_state(&codex_runs, &opencode_runs, &provider_run_id);
                        break;
                    }
                    ProviderRunActorCommand::SyncSelection { provider_run_id } => {
                        let result =
                            execute_selection_sync_command(&opencode_runs, &provider_run_id);
                        let finished = FinishedProviderRunSelectionSyncJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_selection_sync(&finished_selection_syncs, finished);
                    }
                    ProviderRunActorCommand::PollOutput {
                        provider_run_id,
                        run,
                    } => {
                        let result = execute_output_poll_command(&codex_runs, &opencode_runs, &run);
                        clear_structured_output_poll_in_flight(
                            &structured_output_polls,
                            &provider_run_id,
                        );
                        let finished = FinishedProviderOutputPollJob {
                            provider_run_id,
                            result,
                        };
                        push_finished_output_poll(&finished_output_polls, finished);
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
        assert!(mailbox.mark_structured_output_poll_in_flight("run-1".to_string()));
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
        assert!(mailbox
            .structured_output_polls
            .lock()
            .expect("structured output poll set should not be poisoned")
            .is_empty());
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

fn execute_submit_command(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run: RuntimeProviderRun,
    prompt: String,
    attachments: Vec<PromptAttachment>,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(std::time::Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let mut state = codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .remove(&run_id)
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run_id.clone(),
                operation: "codex_thread_missing",
                message: "no Codex thread is bound to this provider run".to_string(),
            })?;
        let result = submit_codex_prompt(&run, &mut state, &prompt, &attachments);
        codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .insert(run_id, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let state = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .remove(&run_id)
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.clone(),
            operation: "opencode_session_missing",
            message: "no OpenCode session is bound to this provider run".to_string(),
        })?;
    let result = submit_opencode_prompt(&run, &state, &prompt, &attachments);
    opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .insert(run_id, state);
    result
}

fn execute_abort_command(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(std::time::Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let mut state = codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .remove(&run_id)
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run_id.clone(),
                operation: "codex_thread_missing",
                message: "no Codex thread is bound to this provider run".to_string(),
            })?;
        let result = abort_codex_turn(&run_id, &mut state);
        codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .insert(run_id, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let state = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .remove(&run_id)
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run_id.clone(),
            operation: "opencode_session_missing",
            message: "no OpenCode session is bound to this provider run".to_string(),
        })?;
    let result = abort_opencode_session(&run_id, &state);
    opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .insert(run_id, state);
    result
}

fn execute_terminate_command(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "codex"
        && !codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .contains_key(&run_id)
    {
        return Ok(());
    }
    if run.adapter_key() == "opencode"
        && !opencode_runs
            .lock()
            .expect("opencode runtime map poisoned")
            .contains_key(&run_id)
    {
        return Ok(());
    }
    execute_abort_command(codex_runs, opencode_runs, run)
}

fn execute_selection_sync_command(
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run_id: &str,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let Some((base_url, session_id)) = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .get(run_id)
        .map(|state| (state.base_url().to_string(), state.session_id().to_string()))
    else {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run_id.to_string(),
            operation: "opencode_session_missing",
            message: "no OpenCode session is bound to this provider run".to_string(),
        });
    };
    sync_opencode_run_selection_for_session(run_id, &base_url, &session_id)
}

fn execute_output_poll_command(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run: &RuntimeProviderRun,
) -> Result<Option<ProviderPromptSignalBatch>, DaemonError> {
    let run_id = run.id();
    if run.adapter_key() == "codex" {
        let Some(poll) = codex_runs
            .lock()
            .expect("codex runtime map poisoned")
            .get_mut(run_id)
            .map(|state| drain_codex_events(run_id, state))
        else {
            return Ok(None);
        };
        let poll = poll?;
        return Ok(Some(ProviderPromptSignalBatch {
            chunks: poll
                .chunks
                .into_iter()
                .map(|chunk| ProviderPromptChunk {
                    kind: chunk.kind,
                    merge_key: chunk.merge_key,
                    bytes: chunk.bytes,
                })
                .collect(),
            completions: poll
                .completions
                .into_iter()
                .map(|completion| ProviderAssistantCompletion {
                    message_id: completion.message_id,
                    completed_at_ms: completion.completed_at_ms,
                })
                .collect(),
            prompt_completed: poll.prompt_completed,
            notices: poll.notices,
            resolved_model: None,
            resolved_model_source: None,
            resolved_variant: None,
            resolved_usage_tokens_total: None,
        }));
    }
    if run.adapter_key() != "opencode" {
        return Ok(None);
    }
    let Some(drain) = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .get_mut(run_id)
        .map(|state| drain_opencode_events(state, run_id))
    else {
        return Ok(None);
    };
    let drain = drain?;
    Ok(Some(ProviderPromptSignalBatch {
        chunks: drain
            .chunks
            .into_iter()
            .map(|chunk| ProviderPromptChunk {
                kind: chunk.kind,
                merge_key: chunk.merge_key,
                bytes: chunk.bytes,
            })
            .collect(),
        completions: drain
            .completions
            .into_iter()
            .map(|completion| ProviderAssistantCompletion {
                message_id: completion.message_id,
                completed_at_ms: completion.completed_at_ms,
            })
            .collect(),
        prompt_completed: drain.prompt_completed,
        notices: drain.notices,
        resolved_model: drain.resolved_model,
        resolved_model_source: drain.resolved_model_source,
        resolved_variant: drain.resolved_variant,
        resolved_usage_tokens_total: drain.resolved_usage_tokens_total,
    }))
}

fn clear_structured_prompt_io_in_flight(
    structured_prompt_submissions: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
) {
    structured_prompt_submissions
        .lock()
        .expect("structured prompt submission set poisoned")
        .remove(run_id);
}

fn clear_structured_output_poll_in_flight(
    structured_output_polls: &Arc<Mutex<BTreeSet<String>>>,
    run_id: &str,
) {
    structured_output_polls
        .lock()
        .expect("structured output poll set poisoned")
        .remove(run_id);
}

fn clear_runtime_state(
    codex_runs: &Arc<Mutex<BTreeMap<String, CodexRuntimeState>>>,
    opencode_runs: &Arc<Mutex<BTreeMap<String, OpenCodeRuntimeState>>>,
    run_id: &str,
) {
    codex_runs
        .lock()
        .expect("codex runtime map poisoned")
        .remove(run_id);
    if let Some(state) = opencode_runs
        .lock()
        .expect("opencode runtime map poisoned")
        .remove(run_id)
    {
        state.stop();
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

fn push_finished_selection_sync(
    finished_selection_syncs: &Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
    finished: FinishedProviderRunSelectionSyncJob,
) {
    match finished_selection_syncs.lock() {
        Ok(mut jobs) => jobs.push(finished),
        Err(error) => {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "provider run selection sync completion queue poisoned",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }
}

fn push_finished_output_poll(
    finished_output_polls: &Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    finished: FinishedProviderOutputPollJob,
) {
    match finished_output_polls.lock() {
        Ok(mut jobs) => jobs.push(finished),
        Err(error) => {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                "provider run output poll completion queue poisoned",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }
}
