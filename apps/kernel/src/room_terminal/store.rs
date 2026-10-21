//! What a kernel restart keeps of a terminal: its metadata and scrollback.

use std::fs;
use std::io::Write;
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
}

impl TerminalStore {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(super) fn save(&self, record: &TerminalRecord) -> Result<(), DaemonError> {
        let io = |error: std::io::Error| store_error(&self.root, error);
        fs::create_dir_all(&self.root).map_err(io)?;
        let path = self.path(&record.terminal_id);
        let temporary = path.with_extension("json.tmp");
        let body = serde_json::to_vec(record).map_err(|error| DaemonError::LocalTransport {
            operation: "save room terminal",
            message: error.to_string(),
        })?;
        let mut file = open_private(&temporary).map_err(io)?;
        file.write_all(&body).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&temporary, &path).map_err(io)
    }

    pub(super) fn remove(&self, terminal_id: &str) {
        let _ = fs::remove_file(self.path(terminal_id));
    }

    /// Every stored record; a record that ran when the kernel stopped is
    /// rewritten as ended by the restart.
    pub(super) fn recover(&self) -> Result<Vec<TerminalRecord>, DaemonError> {
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
            if record.ended.is_none() {
                record.ended = Some(RoomTerminalOutcome::KernelRestarted);
                self.save(&record)?;
            }
            records.push(record);
        }
        Ok(records)
    }

    fn path(&self, terminal_id: &str) -> PathBuf {
        self.root.join(format!("{terminal_id}.json"))
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
