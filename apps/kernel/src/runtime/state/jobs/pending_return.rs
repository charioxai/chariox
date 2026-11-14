//! A remote command that finished while some of its files are still on the worker keeps that
//! worker until they are home, so a relay outage or a full disk cannot destroy the only copy.
use super::remote::Remote;
use super::*;
use serde::{Deserialize, Serialize};

/// What collecting the remaining files needs once the command's own run is over.
#[derive(Serialize, Deserialize)]
pub(super) struct PendingReturn {
    pub(super) root: String,
    pub(super) kernel_id: String,
    pub(super) base: crate::jobs::Manifest,
    pub(super) uploaded: crate::jobs::Manifest,
    /// The command's own outcome, restored once its files are home.
    pub(super) outcome: JobOutcome,
}

impl PendingReturn {
    fn path(state: &KernelRuntimeState, job_id: &str) -> PathBuf {
        state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs")
            .join(job_id)
            .join("return.json")
    }

    pub(super) fn save(&self, state: &KernelRuntimeState, job_id: &str) -> Result<(), DaemonError> {
        let path = Self::path(state, job_id);
        let parent = path.parent().expect("job return plan parent");
        std::fs::create_dir_all(parent).map_err(|e| ledger::error(e.to_string()))?;
        let temporary = parent.join(format!(".return-{:032x}", rand::random::<u128>()));
        std::fs::write(
            &temporary,
            serde_json::to_vec(self).map_err(|e| ledger::error(e.to_string()))?,
        )
        .and_then(|()| std::fs::rename(temporary, path))
        .map_err(|e| ledger::error(format!("Cannot keep the job's file return plan: {e}")))
    }

    fn load(state: &KernelRuntimeState, job_id: &str) -> Result<Self, DaemonError> {
        let bytes = std::fs::read(Self::path(state, job_id))
            .map_err(|e| ledger::error(format!("The job's file return plan is missing: {e}")))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ledger::error("The job's file return plan is unreadable"))
    }
}

impl KernelRuntimeState {
    /// The worker's files are home (or given up): the plan and the live grant are no longer needed.
    pub(super) fn settle_pending_return(&self, record: &mut Record) {
        record.return_pending = false;
        let _ = std::fs::remove_file(PendingReturn::path(self, &record.job_id));
        if let Some(driver) = &self.job_wake_driver {
            driver
                .return_grants
                .lock()
                .expect("job return grants")
                .remove(&record.job_id);
        }
    }

    /// Fetches the files a finished job's worker still holds and records them in its result.
    /// Files an earlier attempt already brought home apply again as no-ops.
    pub(super) async fn collect_pending_return(
        &self,
        record: &mut Record,
    ) -> Result<(), DaemonError> {
        let plan = PendingReturn::load(self, &record.job_id)?;
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let live = driver
            .return_grants
            .lock()
            .expect("job return grants")
            .get(&record.job_id)
            .cloned();
        let grant = match live {
            Some(grant) => grant,
            None => self.job_machine_grant(
                &plan.kernel_id,
                &self
                    .agent_harness_name(&record.room_id, &record.agent_id)
                    .unwrap_or_else(|| "Chariox".into()),
            )?,
        };
        let remote = Remote::new(grant, String::new(), record.job_id.clone());
        let (files, conflicts) = remote
            .return_files(&plan.root, record, &plan.base, &plan.uploaded)
            .await?;
        let result = record
            .result
            .as_mut()
            .ok_or_else(|| ledger::error("The job has no result to complete"))?;
        result.files_back = files;
        result.conflicts = conflicts;
        result.outcome = plan.outcome;
        result
            .output_tail
            .push_str("The files the worker still held were collected.\n");
        self.settle_pending_return(record);
        save_record(self, record)
    }
}
