use std::sync::{Arc, Mutex};
use std::thread;

use crate::error::DaemonError;

use super::{
    ProviderPromptAbortCompletion, ProviderPromptAbortJob, ProviderPromptSubmitCompletion,
    ProviderPromptSubmitJob,
};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunActorMailbox {
    finished_submits: Arc<Mutex<Vec<FinishedProviderPromptSubmitJob>>>,
    finished_aborts: Arc<Mutex<Vec<FinishedProviderPromptAbortJob>>>,
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

impl ProviderRunActorMailbox {
    pub(crate) fn spawn_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        job: ProviderPromptSubmitJob,
    ) {
        let finished_submits = Arc::clone(&self.finished_submits);
        thread::spawn(move || {
            let (completion, result) = job.execute();
            let finished = FinishedProviderPromptSubmitJob {
                session_id,
                provider_run_id,
                agent_id,
                completion,
                result,
            };
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
        });
    }

    pub(crate) fn spawn_abort(
        &self,
        session_id: String,
        provider_run_id: String,
        job: ProviderPromptAbortJob,
    ) {
        let finished_aborts = Arc::clone(&self.finished_aborts);
        thread::spawn(move || {
            let (completion, result) = job.execute();
            let finished = FinishedProviderPromptAbortJob {
                session_id,
                provider_run_id,
                completion,
                result,
            };
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
        });
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
}
