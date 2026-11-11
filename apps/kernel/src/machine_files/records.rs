//! The record each transfer job keeps on disk, and the one writer that puts the records there in
//! the order they were queued: a newer state of a job is never overwritten by an older one, and
//! a caller that must not go on before a state is durable can wait for it.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::local::{FileTransferJob, StartFileTransferRequest};

/// What a job keeps on disk: how it stands and what it was asked to do.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) job: FileTransferJob,
    pub(super) request: StartFileTransferRequest,
}

struct Write {
    record: Option<Record>,
    forget: Vec<String>,
    done: oneshot::Sender<Result<(), String>>,
}

/// The answer to a queued write: it holds once the record is on disk.
pub(super) struct Pending(oneshot::Receiver<Result<(), String>>);

impl Pending {
    pub(super) async fn recorded(self) -> Result<(), String> {
        self.0
            .await
            .unwrap_or_else(|_| Err("the record writer stopped".to_owned()))
    }
}

/// Queues records for the one writer of a folder. Whoever queues a state of a job does so while
/// holding that job's lock, so the queue order is the order the states were taken in.
#[derive(Clone)]
pub(super) struct Recorder {
    queue: mpsc::UnboundedSender<Write>,
}

/// Holds back the first write of a recorder, so a test can make a queued snapshot late.
#[cfg(test)]
pub(super) static FIRST_WRITE_DELAY_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

impl Recorder {
    /// Opens `dir`: the writer runs on the current runtime.
    pub(super) fn open(dir: PathBuf) -> Self {
        let (queue, writes) = mpsc::unbounded_channel();
        tokio::spawn(write_in_order(Arc::new(dir), writes));
        Self { queue }
    }

    pub(super) fn save(&self, record: Record) -> Pending {
        self.send(Some(record), Vec::new())
    }

    pub(super) fn forget(&self, job_ids: Vec<String>) {
        drop(self.send(None, job_ids));
    }

    fn send(&self, record: Option<Record>, forget: Vec<String>) -> Pending {
        let (done, answer) = oneshot::channel();
        // A stopped writer drops the answer, which reads as a failed write.
        let _ = self.queue.send(Write {
            record,
            forget,
            done,
        });
        Pending(answer)
    }
}

async fn write_in_order(dir: Arc<PathBuf>, mut writes: mpsc::UnboundedReceiver<Write>) {
    #[cfg(test)]
    {
        let delay = FIRST_WRITE_DELAY_MS.swap(0, std::sync::atomic::Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
    }
    while let Some(write) = writes.recv().await {
        let (
            path,
            Write {
                record,
                forget,
                done,
            },
        ) = (dir.clone(), write);
        let outcome = tokio::task::spawn_blocking(move || {
            write_to_disk(&path, record.as_ref(), &forget).map_err(|error| error.to_string())
        })
        .await
        .unwrap_or_else(|_| Err("the record writer stopped".to_owned()));
        if let Err(reason) = &outcome {
            crate::logging::warn_with_fields(
                "file_transfer.store",
                "a transfer record could not be written",
                serde_json::json!({"error": reason}),
            );
        }
        let _ = done.send(outcome);
    }
}

fn record_path(dir: &Path, job_id: &str) -> PathBuf {
    dir.join(format!("{job_id}.json"))
}

/// A temporary file renamed into place, so a record is whole or absent.
fn write_to_disk(dir: &Path, record: Option<&Record>, forget: &[String]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    if let Some(record) = record {
        let job_id = &record.job.job_id;
        let temporary = dir.join(format!(".{job_id}.tmp"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(record).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, record_path(dir, job_id))?;
    }
    for job_id in forget {
        let _ = std::fs::remove_file(record_path(dir, job_id));
    }
    Ok(())
}

/// The records of an earlier life of the kernel; one that cannot be read is left out and logged.
pub(super) fn read_records(dir: &Path) -> Vec<Record> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("transfer-") && name.ends_with(".json")
        })
        .filter_map(|entry| {
            std::fs::read(entry.path())
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<Record>(&bytes).map_err(|error| error.to_string())
                })
                .map_err(|error| {
                    crate::logging::warn_with_fields(
                        "file_transfer.store",
                        "a transfer record could not be read and is left out",
                        serde_json::json!({"path": entry.path().display().to_string(), "error": error}),
                    );
                })
                .ok()
        })
        .collect()
}
