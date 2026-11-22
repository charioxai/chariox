use super::*;

use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) job_id: String,
    pub(super) wake_id: String,
    pub(super) registration_id: String,
    pub(super) room_id: String,
    pub(super) agent_id: String,
    pub(super) request: RunJobRequest,
    pub(super) workspace: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) placement: JobPlacement,
    pub(super) started_at_ms: u64,
    #[serde(default)]
    pub(super) running_at_ms: Option<u64>,
    pub(super) state: String,
    pub(super) allocation_ref: Option<String>,
    pub(super) capability_revision: Option<String>,
    pub(super) machine_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) held_kernel_id: Option<String>,
    pub(super) address: Option<String>,
    pub(super) result: Option<JobResult>,
    #[serde(default)]
    pub(super) exec_id: Option<String>,
    #[serde(default)]
    pub(super) class_id: Option<String>,
    #[serde(default)]
    pub(super) rate_per_minute: Option<crate::local::JobMachineRate>,
    #[serde(default)]
    pub(super) estimate_micros: Option<u64>,
    #[serde(default)]
    pub(super) deleted: bool,
    #[serde(default)]
    pub(super) remote_available: bool,
    #[serde(default)]
    pub(super) remote_refusal: Option<String>,
    #[serde(default)]
    pub(super) estimated_minutes: Option<u64>,
    #[serde(default)]
    pub(super) memory_mib: Option<u64>,
    /// Where the job's worker stands from the saved return plan until its files are home.
    #[serde(default)]
    pub(super) remote: RemoteStage,
}

impl Record {
    /// The job asked for, holds or may still hold something on a remote machine.
    pub(super) fn has_remote_trace(&self) -> bool {
        self.placement == JobPlacement::Remote
            || self.allocation_ref.is_some()
            || self.remote != RemoteStage::Idle
    }
}

impl JobDriver {
    pub(super) fn path(&self, id: &str) -> Result<PathBuf, DaemonError> {
        if !id.starts_with("job-") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(ledger::error("invalid job_id"));
        }
        Ok(self
            .state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs")
            .join(format!("{id}.json")))
    }
    pub(super) fn read(&self, id: &str) -> Result<Record, DaemonError> {
        let bytes = std::fs::read(self.path(id)?).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ledger::error("job result is missing")
            } else {
                ledger::error("job result cannot be read")
            }
        })?;
        serde_json::from_slice(&bytes).map_err(|_| ledger::error("job record is unreadable"))
    }
    pub(super) fn save(&self, record: &Record) -> Result<(), DaemonError> {
        save_record(&self.state, record)
    }
    pub(super) fn own(&self, id: &str, task: &AgentTaskExecution) -> Result<Record, DaemonError> {
        self.path(id)?;
        let record = self.read(id).inspect_err(|error| {
            summary::mark_read_error(&self.state, task, id, &error.to_string());
        })?;
        if record.room_id != task.room_id || record.agent_id != task.agent_id {
            return Err(ledger::error("this agent has no such job"));
        }
        Ok(record)
    }
}

pub(super) fn save_record(state: &KernelRuntimeState, record: &Record) -> Result<(), DaemonError> {
    let path = state
        .owned
        .config_projection
        .snapshot()
        .private_runtime_state_root()
        .join("jobs")
        .join(format!("{}.json", record.job_id));
    let bytes = serde_json::to_vec(record).map_err(|error| ledger::error(error.to_string()))?;
    // A list entry advertises a result only after its full body has been saved.
    atomic_write(&path, &bytes)?;
    summary::save(state, record, bytes.len() as u64)?;
    state
        .owned
        .session_projection
        .record_session_change(&record.room_id);
    Ok(())
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    let parent = path.parent().expect("job record parent");
    std::fs::create_dir_all(parent).map_err(|error| ledger::error(error.to_string()))?;
    let temporary = parent.join(format!(".job-{:032x}", rand::random::<u128>()));
    let result = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| ledger::error(error.to_string()))
}

pub(super) fn public_record(record: Record) -> Result<serde_json::Value, DaemonError> {
    Ok(
        serde_json::json!({"job_id":record.job_id,"registration_id":record.registration_id,"placement":record.placement,"state":record.state,"machine":record.machine_label,"address":record.address,"class_id":record.class_id,"rate_per_minute":record.rate_per_minute,"estimate_micros":record.estimate_micros,"deleted":record.deleted,"return_pending":record.remote.holds_worker(),"result":record.result}),
    )
}
