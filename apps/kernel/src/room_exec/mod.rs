//! Binary pipes owned by the Room terminal authority, never a PTY or history.
mod buffer;
pub(crate) mod gateway_placement;
mod launch;
mod lease;
mod place_command;
mod placement;
#[cfg(test)]
mod placement_tests;
mod process;
#[cfg(test)]
pub(crate) mod queue_tests;
mod remote;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Notify};

use crate::error::DaemonError;
use crate::room_terminal::managed_work::ManagedWork;
use crate::runtime::process_resources::{LaunchAdmission, LaunchWait};
use crate::runtime::state::ManagedKernelQuiescenceGate;
pub(crate) use place_command::{
    destinations, offer_placement, run_on_kernel, Placement, Requester,
};

pub const CHUNK_BYTES: usize = 256 * 1024;
const MAX_PER_ROOM: usize = 16;
const MAX_PER_KERNEL: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecOutcome {
    Exited { code: i32 },
    Signaled { signal: i32 },
    MemoryLimit { limit_bytes: u64 },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecState {
    Queued { wait: LaunchWait },
    Starting,
    Running,
    Finished,
}

/// Original intent only. Process state, credentials and inherited environment
/// belong to execution and are never part of a queued launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchIntent {
    pub exec_id: String,
    pub session_id: String,
    pub owner_user_id: String,
    pub acting_agent: Option<String>,
    pub cwd: PathBuf,
    pub command: Vec<String>,
}

impl LaunchIntent {
    pub(crate) fn process_owner(&self) -> crate::runtime::process_resources::SessionProcessOwner {
        use crate::runtime::process_resources::SessionProcessOwner;
        self.acting_agent.as_ref().map_or_else(
            || SessionProcessOwner::Terminal(self.exec_id.clone()),
            |agent| SessionProcessOwner::Agent(agent.clone()),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSummary {
    pub exec_id: String,
    pub state: ExecState,
    pub age_ms: u64,
    pub buffered_bytes: usize,
    pub command_line: String,
    pub command: Vec<String>,
    pub owner: crate::runtime::process_resources::SessionProcessOwner,
    /// Kernels a held launch could run on instead; the router fills it for the caller.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placeable_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecChunk {
    pub cursor: u64,
    pub stream: ExecStream,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutput {
    pub chunks: Vec<ExecChunk>,
    pub next_cursor: u64,
    pub outcome: Option<ExecOutcome>,
}

#[derive(Clone, Copy)]
enum CancelReason {
    Canceled,
    Expired,
}

pub(super) struct Record {
    intent: LaunchIntent,
    launch_command: Vec<String>,
    job: bool,
    minimum_memory: u64,
    state: Mutex<ExecState>,
    force: watch::Sender<bool>,
    place: watch::Sender<Option<Arc<placement::Placed>>>,
    created_at: tokio::time::Instant,
    reader_deadline: watch::Sender<tokio::time::Instant>,
    reader_lease: Duration,
    input: Mutex<Input>,
    output: Mutex<buffer::Output>,
    changed: Notify,
    space: Notify,
    cancel: watch::Sender<Option<CancelReason>>,
}

impl Record {
    fn renew_reader(&self) {
        self.reader_deadline
            .send_replace(tokio::time::Instant::now() + self.reader_lease);
    }
}

struct Input {
    sender: Option<mpsc::Sender<Option<Vec<u8>>>>,
    offset: u64,
    last: Option<(u64, [u8; 32])>,
}

#[derive(Default, Clone)]
pub struct RoomExecService {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    records: Mutex<BTreeMap<String, Arc<Record>>>,
    managed: ManagedWork,
    running: AtomicUsize,
    admission: LaunchAdmission,
}

impl RoomExecService {
    pub(crate) fn link_managed_kernel(
        &self,
        gate: Arc<ManagedKernelQuiescenceGate>,
        callback: std::sync::Weak<dyn Fn() + Send + Sync>,
    ) {
        self.inner.managed.link(gate, callback);
    }

    pub fn has_running_work(&self) -> bool {
        self.inner.running.load(Ordering::Acquire) != 0
    }

    pub(crate) fn report_work(&self) {
        self.inner.managed.report(|| self.has_running_work());
    }

    pub fn open(
        &self,
        session_id: &str,
        cwd: &Path,
        command: &[String],
    ) -> Result<String, DaemonError> {
        self.open_as(
            session_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            cwd,
            command,
            None,
            None,
        )
    }

    pub(crate) fn open_as(
        &self,
        session_id: &str,
        owner_user_id: &str,
        cwd: &Path,
        command: &[String],
        authority: Option<crate::runtime::state::KernelRuntimeState>,
        acting_agent: Option<String>,
    ) -> Result<String, DaemonError> {
        self.open_launch(
            session_id,
            owner_user_id,
            cwd,
            command,
            authority,
            acting_agent,
            false,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_job_as(
        &self,
        session_id: &str,
        owner_user_id: &str,
        cwd: &Path,
        command: &[String],
        needs: &crate::local::JobNeeds,
        authority: Option<crate::runtime::state::KernelRuntimeState>,
        acting_agent: Option<String>,
    ) -> Result<String, DaemonError> {
        self.open_launch(
            session_id,
            owner_user_id,
            cwd,
            command,
            authority,
            acting_agent,
            true,
            u64::from(needs.memory_gib.unwrap_or(0)) * 1024 * 1024 * 1024,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open_launch(
        &self,
        session_id: &str,
        owner_user_id: &str,
        cwd: &Path,
        command: &[String],
        authority: Option<crate::runtime::state::KernelRuntimeState>,
        acting_agent: Option<String>,
        job: bool,
        minimum_memory: u64,
    ) -> Result<String, DaemonError> {
        if command.is_empty()
            || command.len() > 64
            || command[0].is_empty()
            || command
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        {
            return Err(error(
                "exec_invalid",
                "command must contain 1-64 arguments, each at most 4096 bytes",
            ));
        }
        let cwd = cwd.canonicalize().map_err(|failure| {
            error(
                "exec_invalid",
                format!("cannot resolve Room directory: {failure}"),
            )
        })?;
        if !cwd.is_dir() {
            return Err(error("exec_invalid", "Room directory is not a directory"));
        }
        crate::runtime::process_resources::validate_executable(
            &command[0],
            Some(&cwd),
            std::env::var_os("PATH").as_deref(),
        )
        .map_err(|failure| error("exec_spawn_failed", failure.to_string()))?;
        let admission = self.inner.managed.admission()?;
        let mut records = self.inner.records.lock().expect("exec records poisoned");
        if records.len() >= MAX_PER_KERNEL
            || self.inner.running.load(Ordering::Acquire) >= MAX_PER_KERNEL
            || records
                .values()
                .filter(|record| record.intent.session_id == session_id)
                .count()
                >= MAX_PER_ROOM
        {
            return Err(error(
                "exec_limit",
                "use RoomTerminal exec_list to find and exec_close a collected or abandoned command before starting another",
            ));
        }
        let (sender, receiver) = mpsc::channel(8);
        let (cancel, cancelled) = watch::channel(None);
        let created_at = tokio::time::Instant::now();
        let reader_lease = if job {
            lease::JOB_READER_LEASE
        } else {
            lease::READER_LEASE
        };
        let (reader_deadline, deadline) = watch::channel(created_at + reader_lease);
        let exec_id = format!("exec-{:032x}", rand::random::<u128>());
        let (force, forced) = watch::channel(false);
        let (place, placed) = watch::channel(None);
        let record = Arc::new(Record {
            job,
            minimum_memory,
            launch_command: if job {
                vec![
                    "python3".into(),
                    "-c".into(),
                    crate::jobs::COMMAND_WRAPPER.into(),
                ]
            } else {
                command.to_vec()
            },
            intent: LaunchIntent {
                exec_id: exec_id.clone(),
                session_id: session_id.to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                acting_agent,
                cwd,
                command: command.to_vec(),
            },
            state: Mutex::new(ExecState::Starting),
            force,
            place,
            created_at,
            reader_deadline,
            reader_lease,
            input: Mutex::new(Input {
                sender: Some(sender),
                offset: 0,
                last: None,
            }),
            output: Mutex::new(buffer::Output::default()),
            changed: Notify::new(),
            space: Notify::new(),
            cancel,
        });
        records.insert(exec_id.clone(), record.clone());
        self.inner.running.fetch_add(1, Ordering::AcqRel);
        lease::start(exec_id.clone(), Arc::downgrade(&self.inner), deadline);
        launch::start(
            record,
            receiver,
            launch::Signals {
                cancel: cancelled,
                forced,
                placed,
            },
            self.clone(),
            authority,
        );
        drop(records);
        drop(admission);
        self.report_work();
        Ok(exec_id)
    }

    pub fn list(&self, session_id: &str) -> Vec<ExecSummary> {
        self.inner
            .records
            .lock()
            .expect("exec records poisoned")
            .iter()
            .filter(|(_, record)| record.intent.session_id == session_id)
            .map(|(exec_id, record)| {
                let output = record.output.lock().expect("exec output poisoned");
                ExecSummary {
                    exec_id: exec_id.clone(),
                    state: record.state.lock().expect("exec state poisoned").clone(),
                    age_ms: record.created_at.elapsed().as_millis() as u64,
                    buffered_bytes: output.retained_bytes(),
                    command: record.intent.command.clone(),
                    command_line: crate::runtime::process_resources::command_line(
                        &record.intent.command,
                    ),
                    owner: record.intent.process_owner(),
                    placeable_on: Vec::new(),
                }
            })
            .collect()
    }

    fn record(&self, session_id: &str, exec_id: &str) -> Result<Arc<Record>, DaemonError> {
        self.inner
            .records
            .lock()
            .expect("exec records poisoned")
            .get(exec_id)
            .filter(|record| record.intent.session_id == session_id)
            .cloned()
            .ok_or_else(|| {
                error(
                    "exec_not_found",
                    "this Room has no such exec (records expire after five minutes without an output read); use exec_list to find current IDs",
                )
            })
    }

    pub fn write(
        &self,
        session_id: &str,
        exec_id: &str,
        offset: u64,
        encoded: &str,
    ) -> Result<u64, DaemonError> {
        if encoded.len() > CHUNK_BYTES.div_ceil(3) * 4 {
            return Err(error(
                "exec_chunk_too_large",
                "stdin chunks must be at most 256 KiB",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| error("exec_invalid", "stdin must be valid base64"))?;
        if bytes.is_empty() || bytes.len() > CHUNK_BYTES {
            return Err(error(
                "exec_chunk_too_large",
                "stdin chunks must contain 1-262144 bytes; close stdin to send EOF",
            ));
        }
        let record = self.record(session_id, exec_id)?;
        let mut input = record.input.lock().expect("exec input poisoned");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if input.last == Some((offset, digest)) {
            return Ok(input.offset);
        }
        if offset != input.offset {
            return Err(error(
                "exec_offset_mismatch",
                format!("stdin expects offset {}; resume there", input.offset),
            ));
        }
        let next = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("exec_offset_mismatch", "stdin offset overflow"))?;
        let closed = || {
            let output = record.output.lock().expect("exec output poisoned");
            let message = match &output.outcome {
                Some(ExecOutcome::Failed { message }) => message.as_str(),
                _ => "stdin is closed; start a new exec to send more input",
            };
            error("exec_stdin_closed", message)
        };
        let sender = input.sender.as_ref().ok_or_else(closed)?;
        sender
            .try_send(Some(bytes))
            .map_err(|failure| match failure {
                mpsc::error::TrySendError::Closed(_) => closed(),
                failure => input_error(failure),
            })?;
        input.offset = next;
        input.last = Some((offset, digest));
        Ok(next)
    }

    pub fn close_stdin(
        &self,
        session_id: &str,
        exec_id: &str,
        offset: u64,
    ) -> Result<u64, DaemonError> {
        let record = self.record(session_id, exec_id)?;
        let mut input = record.input.lock().expect("exec input poisoned");
        if offset != input.offset {
            return Err(error(
                "exec_offset_mismatch",
                format!(
                    "stdin expects offset {}; close at that offset",
                    input.offset
                ),
            ));
        }
        if let Some(sender) = &input.sender {
            // A reaped child may close its receiver before cleanup publishes
            // the outcome. Closing is already satisfied; readers still drain it.
            match sender.try_send(None) {
                Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
                Err(failure) => return Err(input_error(failure)),
            }
            input.sender = None;
        }
        Ok(offset)
    }

    pub async fn read(
        &self,
        session_id: &str,
        exec_id: &str,
        after_cursor: u64,
        wait: Duration,
    ) -> Result<ExecOutput, DaemonError> {
        let record = self.record(session_id, exec_id)?;
        record.renew_reader();
        let deadline = tokio::time::Instant::now() + wait.min(crate::room_terminal::MAX_WAIT);
        loop {
            // Register before inspecting state, so output cannot race the wait.
            let changed = record.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let page = record
                .output
                .lock()
                .expect("exec output poisoned")
                .page(after_cursor)?;
            record.space.notify_waiters();
            if !page.chunks.is_empty()
                || page.outcome.is_some()
                || tokio::time::Instant::now() >= deadline
            {
                return Ok(page);
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return record
                    .output
                    .lock()
                    .expect("exec output poisoned")
                    .page(after_cursor);
            }
        }
    }

    pub fn start_now(&self, session_id: &str, exec_id: &str) -> Result<(), DaemonError> {
        let record = self.record(session_id, exec_id)?;
        let state = record.state.lock().expect("exec state poisoned");
        if !matches!(*state, ExecState::Queued { .. }) {
            return Err(error(
                "exec_not_queued",
                "only a launch held before start can be started now",
            ));
        }
        record.force.send_replace(true);
        Ok(())
    }

    /// Whether `requester` may run this launch on another kernel: it is still held, and an
    /// agent asks only for the launches it started itself.
    pub(crate) fn ensure_placeable_by(
        &self,
        session_id: &str,
        exec_id: &str,
        requester: &place_command::Requester,
    ) -> Result<(), DaemonError> {
        let record = self.record(session_id, exec_id)?;
        if !matches!(
            *record.state.lock().expect("exec state poisoned"),
            ExecState::Queued { .. }
        ) {
            return Err(not_placeable());
        }
        requester.ensure_owns(record.intent.acting_agent.as_deref())
    }

    /// Runs a launch that is still held on another kernel. A process that
    /// already started keeps running where it is.
    pub(crate) fn place(
        &self,
        session_id: &str,
        exec_id: &str,
        origin_kernel: &str,
        target: Arc<dyn placement::ExecPlacement>,
    ) -> Result<(), DaemonError> {
        let record = self.record(session_id, exec_id)?;
        let mut state = record.state.lock().expect("exec state poisoned");
        if !matches!(*state, ExecState::Queued { .. }) {
            return Err(not_placeable());
        }
        // Not queued any more: the held launch must not start here, and it
        // reads as starting until the other kernel reports it running.
        *state = ExecState::Starting;
        record.place.send_replace(Some(Arc::new(placement::Placed {
            target,
            attributed_agent: record
                .intent
                .acting_agent
                .as_ref()
                .map(|agent| format!("{origin_kernel}/{agent}")),
        })));
        record.changed.notify_waiters();
        Ok(())
    }

    pub(crate) fn cancel_queued(
        &self,
        session_id: &str,
        exec_id: &str,
        before_cancel: impl FnOnce(),
    ) -> Result<(), DaemonError> {
        let record = self.record(session_id, exec_id)?;
        let state = record.state.lock().expect("exec state poisoned");
        if !matches!(*state, ExecState::Queued { .. }) {
            return Err(error(
                "job_already_started",
                "This command already started and cannot be moved safely",
            ));
        }
        before_cancel();
        record.cancel.send_replace(Some(CancelReason::Canceled));
        Ok(())
    }

    pub fn cancel(&self, session_id: &str, exec_id: &str) -> Result<(), DaemonError> {
        let record = self.record(session_id, exec_id)?;
        let _state = record.state.lock().expect("exec state poisoned");
        record.cancel.send_replace(Some(CancelReason::Canceled));
        Ok(())
    }

    pub fn close(&self, session_id: &str, exec_id: &str) -> Result<(), DaemonError> {
        let mut records = self.inner.records.lock().expect("exec records poisoned");
        if records
            .get(exec_id)
            .is_some_and(|record| record.intent.session_id != session_id)
        {
            return Err(error("exec_not_found", "this Room has no such exec"));
        }
        if let Some(record) = records.remove(exec_id) {
            record.cancel.send_replace(Some(CancelReason::Canceled));
        }
        drop(records);
        self.report_work();
        Ok(())
    }

    pub fn end_session(&self, session_id: &str) {
        self.remove_where(|record| record.intent.session_id == session_id);
    }

    pub fn shutdown(&self) {
        self.remove_where(|_| true);
    }

    fn finish(&self, record: &Record, outcome: ExecOutcome) {
        let mut output = record.output.lock().expect("exec output poisoned");
        output.outcome = Some(outcome);
        *record.state.lock().expect("exec state poisoned") = ExecState::Finished;
        record.input.lock().expect("exec input poisoned").sender = None;
        self.inner.running.fetch_sub(1, Ordering::AcqRel);
        // A terminal page also promises the launch no longer counts as running work.
        drop(output);
        record.changed.notify_waiters();
        self.report_work();
    }

    fn remove_where(&self, predicate: impl Fn(&Record) -> bool) {
        self.inner
            .records
            .lock()
            .expect("exec records poisoned")
            .retain(|_, record| {
                if predicate(record) {
                    record.cancel.send_replace(Some(CancelReason::Canceled));
                    false
                } else {
                    true
                }
            });
        self.report_work();
    }
}

fn input_error(failure: mpsc::error::TrySendError<Option<Vec<u8>>>) -> DaemonError {
    match failure {
        mpsc::error::TrySendError::Full(_) => error(
            "exec_input_busy",
            "stdin buffer is full; drain output and retry the same input offset",
        ),
        mpsc::error::TrySendError::Closed(_) => error(
            "exec_stdin_closed",
            "the command closed stdin; drain output and collect its outcome",
        ),
    }
}

fn not_placeable() -> DaemonError {
    error(
        "exec_not_queued",
        "only a launch held before start can run on another kernel; a started process keeps running where it is",
    )
}

pub(super) fn error(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.to_string(),
        message: message.into(),
    }
}
