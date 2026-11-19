//! What a kernel restart keeps of a terminal: its metadata and scrollback.
mod writer;
pub(crate) use writer::PendingWrite;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{RoomTerminalOutcome, TerminalActor, TerminalScope};
use crate::error::DaemonError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TerminalRecord {
    pub terminal_id: String,
    #[serde(flatten)]
    pub scope: TerminalScope,
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
        for entry in entries {
            let entry = entry.map_err(|error| store_error(&self.root, error))?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("json"))
                || !entry
                    .file_type()
                    .map_err(|error| store_error(&path, error))?
                    .is_file()
            {
                continue;
            }
            let (mut record, migrated) = match read_for_recovery(&path) {
                Ok(record) => record,
                Err(reason) => {
                    self.quarantine(&path, &reason)?;
                    continue;
                }
            };
            let ended_by_restart =
                record.ended.is_none() && !handed_over.contains(&record.terminal_id);
            if ended_by_restart {
                record.ended = Some(RoomTerminalOutcome::KernelRestarted);
            }
            if migrated || ended_by_restart {
                self.save(&record)?;
            }
            records.push(record);
        }
        Ok(records)
    }

    fn quarantine(&self, path: &Path, reason: &str) -> Result<(), DaemonError> {
        let directory = self.root.join("unreadable");
        fs::create_dir_all(&directory).map_err(|error| store_error(&directory, error))?;
        let name = path.file_name().expect("path came from a directory entry");
        let mut destination = directory.join(name);
        if destination.symlink_metadata().is_ok() {
            destination = directory.join(format!(
                "{}-{}",
                uuid::Uuid::new_v4(),
                name.to_string_lossy()
            ));
        }
        fs::rename(path, &destination).map_err(|error| store_error(path, error))?;
        crate::logging::error_with_fields(
            "room_terminal.recover",
            "unreadable terminal record moved aside",
            serde_json::json!({
                "file": path.display().to_string(),
                "destination": destination.display().to_string(),
                "reason": reason,
            }),
        );
        Ok(())
    }
}

fn read_for_recovery(path: &Path) -> Result<(TerminalRecord, bool), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid JSON at line {}, column {}",
            error.line(),
            error.column()
        )
    })?;
    // Write the scope through the normal atomic writer during startup so
    // subsequent reads use the same record type as newly created terminals.
    let migrated = value.as_object_mut().is_some_and(|fields| {
        if fields.contains_key("scope")
            || !fields
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !id.is_empty())
        {
            return false;
        }
        fields.insert("scope".into(), "room".into());
        true
    });
    let record: TerminalRecord =
        serde_json::from_value(value).map_err(|_| "invalid terminal record fields".to_string())?;
    if path.file_stem().and_then(OsStr::to_str) != Some(record.terminal_id.as_str()) {
        return Err("terminal ID does not match its filename".into());
    }
    Ok((record, migrated))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutover_recovers_and_rewrites_pre_l12_room_terminal() {
        let root = crate::test_support::TestWorktree::new("pre-l12-room-terminal");
        let path = root.path().join("term-old.json");
        let legacy = serde_json::json!({
            "terminal_id": "term-old",
            "session_id": "room-old",
            "title": "fixture terminal",
            "program": "sh",
            "working_directory": "/fixture",
            "cols": 80,
            "rows": 24,
            "opened_by": { "kind": "user" },
            "command": null,
            "user_interacted": true,
            "created_at_ms": 42,
            "ended": null,
            "scrollback_text": "fixture scrollback",
            "screen_text": "fixture screen"
        });
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let store = TerminalStore::new(root.path().to_path_buf()).unwrap();

        let records = store.recover(&Default::default()).unwrap();
        let [record] = records.as_slice() else {
            panic!("the pre-L12 terminal was not recovered");
        };
        assert_eq!(record.scope, TerminalScope::Room("room-old".into()));
        assert_eq!(record.ended, Some(RoomTerminalOutcome::KernelRestarted));
        assert_eq!(record.scrollback_text, "fixture scrollback");
        assert_eq!(record.screen_text, "fixture screen");

        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["scope"], "room");
        assert_eq!(stored["session_id"], "room-old");
        assert_eq!(stored["ended"]["reason"], "kernel_restarted");
        assert_eq!(store.recover(&Default::default()).unwrap().len(), 1);
    }

    #[test]
    fn cutover_quarantines_unreadable_terminal_without_removing_its_bytes() {
        let root = crate::test_support::TestWorktree::new("unreadable-room-terminal");
        let path = root.path().join("term-bad.json");
        let body = b"{\"terminal_id\": \"term-bad\", broken";
        fs::write(&path, body).unwrap();

        let store = TerminalStore::new(root.path().to_path_buf()).unwrap();
        assert!(store.recover(&Default::default()).unwrap().is_empty());
        assert!(!path.exists());
        assert_eq!(
            fs::read(root.path().join("unreadable/term-bad.json")).unwrap(),
            body
        );
        assert!(store.recover(&Default::default()).unwrap().is_empty());
    }
}
