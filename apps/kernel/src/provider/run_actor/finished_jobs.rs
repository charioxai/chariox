use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::provider::opencode_binding::OpenCodeRunSelection;
use crate::provider::{ProviderPromptSignalBatch, ProviderResumeState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderPromptSubmitAcknowledgement {
    pub(crate) resume_state: ProviderResumeState,
}

/// What the provider is known to have done with a submitted prompt. Only
/// `Rejected` proves the prompt was not taken; any other failure may follow a
/// provider write and must not be replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderPromptSubmitReceipt {
    Accepted,
    Rejected,
    Uncertain,
}

impl ProviderPromptSubmitReceipt {
    pub(super) fn from_result(
        result: &Result<ProviderPromptSubmitAcknowledgement, DaemonError>,
    ) -> Self {
        match result {
            Ok(_) => Self::Accepted,
            Err(DaemonError::ProviderPromptSteerRejected { .. }) => Self::Rejected,
            Err(_) => Self::Uncertain,
        }
    }
}

pub(crate) struct FinishedProviderPromptSubmitJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt_id: String,
    pub(crate) result: Result<ProviderPromptSubmitAcknowledgement, DaemonError>,
    pub(crate) settlement_retry_attempt: u32,
}

pub(crate) struct FinishedProviderPromptAbortJob {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) prompt_id: String,
    pub(crate) result: Result<(), DaemonError>,
}

pub(crate) struct FinishedProviderRunSelectionSyncJob {
    pub(crate) provider_run_id: String,
    pub(crate) result: Result<OpenCodeRunSelection, DaemonError>,
}

pub(crate) struct FinishedProviderOutputPollJob {
    pub(crate) provider_run_id: String,
    pub(crate) result: Result<Option<ProviderPromptSignalBatch>, DaemonError>,
    pub(crate) settlement_retry_attempt: u32,
}

pub(super) fn push_finished_submit(
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

pub(super) fn push_finished_abort(
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

pub(super) fn push_finished_selection_sync(
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

pub(super) fn push_finished_output_poll(
    finished_output_polls: &Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    finished: FinishedProviderOutputPollJob,
) {
    crate::logging::debug_with_fields(
        "daemon.provider_run_actor",
        "finished output poll pushed",
        serde_json::json!({
            "provider_run_id": finished.provider_run_id,
            "result_kind": match &finished.result {
                Ok(Some(batch)) => serde_json::json!({
                    "type": "batch",
                    "chunks": batch.chunks.len(),
                    "completions": batch.completions.len(),
                    "prompt_completed": batch.prompt_completed,
                }),
                Ok(None) => serde_json::json!({ "type": "none" }),
                Err(error) => serde_json::json!({ "type": "error", "error": error.to_string() }),
            },
        }),
    );
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

pub(super) fn drain_finished_submits(
    finished_submits: &Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
) -> Vec<FinishedProviderPromptSubmitJob> {
    drain_finished_jobs(
        finished_submits,
        "structured prompt submit completion queue poisoned",
    )
}

pub(super) fn drain_finished_aborts(
    finished_aborts: &Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
) -> Vec<FinishedProviderPromptAbortJob> {
    drain_finished_jobs(
        finished_aborts,
        "structured prompt abort completion queue poisoned",
    )
}

pub(super) fn drain_finished_selection_syncs(
    finished_selection_syncs: &Arc<Mutex<Vec<FinishedProviderRunSelectionSyncJob>>>,
) -> Vec<FinishedProviderRunSelectionSyncJob> {
    drain_finished_jobs(
        finished_selection_syncs,
        "provider run selection sync completion queue poisoned",
    )
}

pub(super) fn drain_finished_output_polls(
    finished_output_polls: &Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
) -> Vec<FinishedProviderOutputPollJob> {
    drain_finished_jobs(
        finished_output_polls,
        "provider run output poll completion queue poisoned",
    )
}

fn drain_finished_jobs<T>(jobs: &Arc<Mutex<Vec<T>>>, poison_message: &'static str) -> Vec<T> {
    match jobs.lock() {
        Ok(mut jobs) => std::mem::take(&mut *jobs),
        Err(error) => {
            crate::logging::error_with_fields(
                "daemon.provider_run_actor",
                poison_message,
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            Vec::new()
        }
    }
}

// MP-08 / MP-10: a run's output lane cannot consume another run's batch.
pub(super) fn drain_finished_output_polls_for_run(
    jobs: &Arc<Mutex<Vec<FinishedProviderOutputPollJob>>>,
    run_id: &str,
) -> Vec<FinishedProviderOutputPollJob> {
    let Ok(mut jobs) = jobs.lock() else {
        crate::logging::error_with_fields(
            "daemon.provider_run_actor",
            "provider run output poll completion queue poisoned",
            serde_json::json!({}),
        );
        return Vec::new();
    };
    let (selected, retained) = std::mem::take(&mut *jobs)
        .into_iter()
        .partition(|job| job.provider_run_id == run_id);
    *jobs = retained;
    selected
}
