//! A remote command's worker is kept from before the command runs until its files are home, so a
//! relay outage or a full disk cannot destroy the only copy.
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

    pub(super) fn load(state: &KernelRuntimeState, job_id: &str) -> Result<Self, DaemonError> {
        let bytes = std::fs::read(Self::path(state, job_id))
            .map_err(|e| ledger::error(format!("The job's file return plan is missing: {e}")))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ledger::error("The job's file return plan is unreadable"))
    }
}

impl KernelRuntimeState {
    /// The worker's plan and live grant are no longer needed.
    pub(super) fn forget_pending_return(&self, job_id: &str) {
        let _ = std::fs::remove_file(PendingReturn::path(self, job_id));
        if let Some(driver) = &self.job_wake_driver {
            driver
                .return_grants
                .lock()
                .expect("job return grants")
                .remove(job_id);
        }
    }

    /// The command is stopped or its files are given up; the worker may now be released.
    pub(super) fn give_up_pending_return(&self, record: &mut Record) {
        if let Ok(next) = record.remote.next(StageEvent::GiveUp) {
            record.remote = next;
        }
        self.forget_pending_return(&record.job_id);
    }

    /// Fetches the files of a finished command and records them in its result. Files an earlier
    /// attempt already brought home apply again as no-ops.
    pub(super) async fn collect_pending_return(
        &self,
        record: &mut Record,
    ) -> Result<(), DaemonError> {
        let RemoteStage::Finished { outcome } = record.remote else {
            return Err(ledger::error(
                "The job's command has not ended; its files are collected once it has",
            ));
        };
        let plan = PendingReturn::load(self, &record.job_id)?;
        let grant = self.return_grant(record, &plan.kernel_id).await?;
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
        result.outcome = outcome;
        result
            .output_tail
            .push_str("The files the worker still held were collected.\n");
        record.advance(StageEvent::Home)?;
        save_record(self, record)?;
        self.forget_pending_return(&record.job_id);
        Ok(())
    }

    /// The live grant to the job's worker, or one rebuilt from the saved allocation after a restart.
    pub(super) async fn return_grant(
        &self,
        record: &Record,
        kernel_id: &str,
    ) -> Result<Arc<dyn crate::room_exec::gateway_placement::GrantedSend>, DaemonError> {
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
        match live {
            Some(grant) => Ok(grant),
            None => {
                self.job_machine_reattach(
                    &record.job_id,
                    record
                        .allocation_ref
                        .as_deref()
                        .ok_or_else(|| ledger::error("The kept job has no allocation"))?,
                    kernel_id,
                    &self
                        .agent_harness_name(&record.room_id, &record.agent_id)
                        .unwrap_or_else(|| "Chariox".into()),
                )
                .await
            }
        }
    }
}
