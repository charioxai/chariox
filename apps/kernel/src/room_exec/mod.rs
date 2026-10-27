//! Binary pipes owned by the Room terminal authority, never a PTY or history.
mod buffer;
mod lease;
mod process;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Notify};

use crate::error::DaemonError;
use crate::room_terminal::managed_work::ManagedWork;
use crate::runtime::state::ManagedKernelQuiescenceGate;

pub const CHUNK_BYTES: usize = crate::local::KERNEL_FILE_MAX_CHUNK_BYTES as usize;
const MAX_PER_ROOM: usize = 16;
const MAX_PER_KERNEL: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecOutcome {
    Exited { code: i32 },
    Signaled { signal: i32 },
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecState {
    Running,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSummary {
    pub exec_id: String,
    pub state: ExecState,
    pub age_ms: u64,
    pub buffered_bytes: usize,
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
    session_id: String,
    created_at: tokio::time::Instant,
    reader_deadline: watch::Sender<tokio::time::Instant>,
    input: Mutex<Input>,
    output: Mutex<buffer::Output>,
    changed: Notify,
    space: Notify,
    cancel: watch::Sender<Option<CancelReason>>,
}

impl Record {
    fn renew_reader(&self) {
        self.reader_deadline
            .send_replace(tokio::time::Instant::now() + lease::READER_LEASE);
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
        let admission = self.inner.managed.admission()?;
        let mut records = self.inner.records.lock().expect("exec records poisoned");
        if records.len() >= MAX_PER_KERNEL
            || self.inner.running.load(Ordering::Acquire) >= MAX_PER_KERNEL
            || records
                .values()
                .filter(|record| record.session_id == session_id)
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
        let (reader_deadline, deadline) = watch::channel(created_at + lease::READER_LEASE);
        let record = Arc::new(Record {
            session_id: session_id.to_owned(),
            created_at,
            reader_deadline,
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
        let child = process::spawn(cwd, command)?;
        let exec_id = format!("exec-{:032x}", rand::random::<u128>());
        records.insert(exec_id.clone(), record.clone());
        self.inner.running.fetch_add(1, Ordering::AcqRel);
        lease::start(exec_id.clone(), Arc::downgrade(&self.inner), deadline);
        process::run(child, record, receiver, cancelled, self.clone());
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
            .filter(|(_, record)| record.session_id == session_id)
            .map(|(exec_id, record)| {
                let output = record.output.lock().expect("exec output poisoned");
                ExecSummary {
                    exec_id: exec_id.clone(),
                    state: if output.outcome.is_some() {
                        ExecState::Finished
                    } else {
                        ExecState::Running
                    },
                    age_ms: record.created_at.elapsed().as_millis() as u64,
                    buffered_bytes: output.retained_bytes(),
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
            .filter(|record| record.session_id == session_id)
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
        let sender = input.sender.as_ref().ok_or_else(|| {
            error(
                "exec_stdin_closed",
                "stdin is closed; start a new exec to send more input",
            )
        })?;
        sender.try_send(Some(bytes)).map_err(input_error)?;
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
            sender.try_send(None).map_err(input_error)?;
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

    pub fn cancel(&self, session_id: &str, exec_id: &str) -> Result<(), DaemonError> {
        self.record(session_id, exec_id)?
            .cancel
            .send_replace(Some(CancelReason::Canceled));
        Ok(())
    }

    pub fn close(&self, session_id: &str, exec_id: &str) -> Result<(), DaemonError> {
        let mut records = self.inner.records.lock().expect("exec records poisoned");
        if records
            .get(exec_id)
            .is_some_and(|record| record.session_id != session_id)
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
        self.remove_where(|record| record.session_id == session_id);
    }

    pub fn shutdown(&self) {
        self.remove_where(|_| true);
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

pub(super) fn error(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.to_string(),
        message: message.into(),
    }
}
