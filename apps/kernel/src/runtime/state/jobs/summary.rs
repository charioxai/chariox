//! Small, room-scoped durable headers. Result bodies are opened by status, never by list reads.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_SUMMARY_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct Summary {
    pub(super) view: crate::local::JobView,
    workspace: PathBuf,
    cwd: PathBuf,
    command_hash: String,
    record_bytes: u64,
    record_modified_ns: String,
}

fn directory(state: &KernelRuntimeState, room: &str) -> PathBuf {
    let room = format!("{:x}", Sha256::digest(room.as_bytes()));
    state
        .owned
        .config_projection
        .snapshot()
        .private_runtime_state_root()
        .join("jobs/summaries")
        .join(room)
}

pub(super) fn command_hash(command: &crate::local::JobCommand) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(command).expect("job command"))
    )
}

impl Summary {
    pub(super) fn new(record: &Record, record_bytes: u64, record_modified_ns: String) -> Self {
        Self {
            view: crate::local::JobView {
                job_id: record.job_id.clone(),
                agent_id: record.agent_id.clone(),
                command: validate(&record.request)
                    .map(|argv| crate::runtime::process_resources::command_line(&argv))
                    .unwrap_or_else(|_| "Job".into())
                    .chars()
                    .take(200)
                    .collect(),
                state: record.state.clone(),
                placement: record.placement,
                started_at_ms: record.started_at_ms,
                machine_label: record.machine_label.clone(),
                address: record.address.clone(),
                class_id: record.class_id.clone(),
                rate_per_minute: record.rate_per_minute.clone(),
                estimate_micros: record.estimate_micros,
                deleted: record.deleted,
                result: record
                    .result
                    .as_ref()
                    .map(|result| crate::local::JobResultSummary {
                        exit_code: result.exit_code,
                        reason: (record.placement == JobPlacement::Remote
                            && record.machine_label.is_none())
                        .then(|| result.output_tail.chars().take(512).collect()),
                        files_back_count: result.files_back.len() as u64,
                        conflicts_count: result.conflicts.len() as u64,
                        ran_on: result.ran_on.clone(),
                        duration_ms: result.duration_ms,
                        outcome: result.outcome,
                    }),
                read_error: None,
            },
            workspace: record.workspace.clone(),
            cwd: record.cwd.clone(),
            command_hash: command_hash(&record.request.command),
            record_bytes,
            record_modified_ns,
        }
    }

    pub(super) fn matches(&self, record: &Record, command_hash: &str) -> bool {
        self.workspace == record.workspace
            && self.cwd == record.cwd
            && self.command_hash == command_hash
    }
}

pub(super) fn save(
    state: &KernelRuntimeState,
    record: &Record,
    bytes: u64,
) -> Result<(), DaemonError> {
    if !record
        .agent_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || record.agent_id.is_empty()
    {
        return Err(ledger::error("invalid job agent_id"));
    }
    let path = directory(state, &record.room_id)
        .join(&record.agent_id)
        .join(format!("{}.json", record.job_id));
    let body = path_for_body(state, &record.job_id);
    let modified =
        modified_ns(&std::fs::metadata(body).map_err(|error| ledger::error(error.to_string()))?)
            .map_err(|error| ledger::error(error.to_string()))?;
    super::record::atomic_write(
        &path,
        &serde_json::to_vec(&Summary::new(record, bytes, modified))
            .map_err(|error| ledger::error(error.to_string()))?,
    )
}

pub(super) fn read(state: &KernelRuntimeState, room: &str) -> Vec<Summary> {
    let Ok(entries) = std::fs::read_dir(directory(state, room)) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .flat_map(|agent| {
            std::fs::read_dir(agent.path())
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
        })
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| {
            let path = entry.path();
            let id = path
                .file_stem()
                .and_then(|id| id.to_str())
                .unwrap_or_default();
            let agent = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|id| id.to_str())
                .unwrap_or_default();
            let mut summary = read_header(&path)
                .filter(|summary| summary.view.job_id == id && summary.view.agent_id == agent)
                .unwrap_or_else(|| unreadable(id, agent));
            let body = path_for_body(state, id);
            // Presence, size and modification checks never open a potentially large or damaged result.
            match std::fs::metadata(body) {
                Ok(metadata)
                    if metadata.is_file()
                        && metadata.len() == summary.record_bytes
                        && modified_ns(&metadata)
                            .is_ok_and(|stamp| stamp == summary.record_modified_ns) => {}
                Ok(_) => {
                    summary.view.read_error =
                        Some("Job result is unreadable; open it to check the saved result".into())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    summary.view.read_error = Some("Job result is missing".into());
                }
                Err(_) => summary.view.read_error = Some("Job result cannot be read".into()),
            }
            summary
        })
        .collect()
}

fn unreadable(id: &str, agent: &str) -> Summary {
    Summary {
        view: crate::local::JobView {
            job_id: id.into(),
            agent_id: agent.into(),
            command: "Job".into(),
            state: "finished".into(),
            placement: JobPlacement::Local,
            started_at_ms: 0,
            machine_label: None,
            address: None,
            class_id: None,
            rate_per_minute: None,
            estimate_micros: None,
            deleted: false,
            result: None,
            read_error: Some(
                "Job summary is unreadable; open the job to check its saved result".into(),
            ),
        },
        workspace: PathBuf::new(),
        cwd: PathBuf::new(),
        command_hash: String::new(),
        record_bytes: 0,
        record_modified_ns: String::new(),
    }
}

pub(super) fn mark_read_error(
    state: &KernelRuntimeState,
    task: &AgentTaskExecution,
    id: &str,
    error: &str,
) {
    let path = directory(state, &task.room_id)
        .join(&task.agent_id)
        .join(format!("{id}.json"));
    let summary = read_header(&path);
    let Some(mut summary) = summary else {
        return;
    };
    summary.view.read_error = Some(error.to_owned());
    match serde_json::to_vec(&summary)
        .map_err(|error| ledger::error(error.to_string()))
        .and_then(|bytes| super::record::atomic_write(&path, &bytes))
    {
        Ok(()) => state
            .owned
            .session_projection
            .record_session_change(&task.room_id),
        Err(error) => tracing::warn!(%error, "job summary could not record the unreadable result"),
    }
}

fn path_for_body(state: &KernelRuntimeState, id: &str) -> PathBuf {
    state
        .owned
        .config_projection
        .snapshot()
        .private_runtime_state_root()
        .join("jobs")
        .join(format!("{id}.json"))
}

fn modified_ns(metadata: &std::fs::Metadata) -> std::io::Result<String> {
    metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .map_err(std::io::Error::other)
}

fn read_header(path: &Path) -> Option<Summary> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SUMMARY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SUMMARY_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
