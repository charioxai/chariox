//! What a kernel restart keeps of a terminal: its metadata and scrollback.
mod writer;
pub(crate) use writer::PendingWrite;

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::{RoomTerminalOutcome, TerminalActor};
use crate::error::DaemonError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TerminalRecord {
    pub terminal_id: String,
    pub session_id: String,
    pub title: String,
    pub program: String,
    pub working_directory: String,
    pub cols: u16,
    pub rows: u16,
    pub opened_by: TerminalActor,
    /// The program the terminal was opened with instead of a login shell.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub user_interacted: bool,
    pub created_at_ms: u64,
    /// `None` while the terminal ran when the record was written.
    pub ended: Option<RoomTerminalOutcome>,
    pub scrollback_text: String,
    pub screen_text: String,
}

pub(super) struct TerminalStore {
    root: PathBuf,
    writer: std::sync::Arc<writer::Writer>,
}

impl TerminalStore {
    pub(super) fn new(root: PathBuf) -> Result<Self, DaemonError> {
        let writer = writer::Writer::new(root.clone())?;
        Ok(Self { root, writer })
    }

    #[cfg(test)]
    pub(super) fn pause_writer(
        &self,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) -> PendingWrite {
        self.writer
            .submit(writer::Operation::Pause { entered, release }, false)
            .unwrap()
    }

    pub(super) fn enqueue_save(
        &self,
        record: &TerminalRecord,
    ) -> Result<PendingWrite, DaemonError> {
        let body = serde_json::to_vec(record).map_err(|error| DaemonError::LocalTransport {
            operation: "save room terminal",
            message: error.to_string(),
        })?;
        self.writer.submit(
            writer::Operation::Save {
                id: record.terminal_id.clone(),
                body,
            },
            false,
        )
    }

    pub(super) fn save(&self, record: &TerminalRecord) -> Result<(), DaemonError> {
        self.enqueue_save(record)?.wait()
    }

    pub(super) fn remove(&self, terminal_id: &str) -> Result<(), DaemonError> {
        self.writer
            .submit(
                writer::Operation::Remove {
                    id: terminal_id.into(),
                },
                true,
            )?
            .wait()
    }

    /// Every stored record; a record that ran when the kernel stopped is
    /// rewritten as ended by the restart, unless a hot reload handed it over.
    pub(super) fn recover(
        &self,
        handed_over: &std::collections::BTreeSet<String>,
    ) -> Result<Vec<TerminalRecord>, DaemonError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(store_error(&self.root, error)),
        };
        let mut records = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension != "json")
            {
                let _ = fs::remove_file(&path);
                continue;
            }
            let record = fs::read(&path)
                .ok()
                .and_then(|body| serde_json::from_slice::<TerminalRecord>(&body).ok());
            let Some(mut record) = record else {
                crate::logging::error_with_fields(
                    "room_terminal.recover",
                    "unreadable terminal record removed",
                    serde_json::json!({ "file": entry.file_name().to_string_lossy() }),
                );
                let _ = fs::remove_file(&path);
                continue;
            };
            if record.ended.is_none() && !handed_over.contains(&record.terminal_id) {
                record.ended = Some(RoomTerminalOutcome::KernelRestarted);
                self.save(&record)?;
            }
            records.push(record);
        }
        Ok(records)
    }
}

fn open_private(path: &std::path::Path) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        fs::File::create(path)
    }
}

fn store_error(root: &std::path::Path, error: std::io::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room terminal store",
        message: format!("{}: {error}", root.display()),
    }
}
