//! Kernel storage full. A full disk refuses durable writes; it does not stop
//! the writer.
//!
//! SQLite reports ENOSPC as `SQLITE_FULL`. The writer runs in WAL mode, and a
//! COMMIT that fails with `SQLITE_FULL` could not append all of its frames:
//! the commit frame is missing or has a bad checksum, and SQLite rolls the
//! transaction back. So that outcome is known (nothing committed), and a
//! caller can report an ordinary storage failure instead of fencing the
//! writer until a restart. Failures after the commit frame is written (a
//! failed fsync, or wal-index growth) are I/O errors, not `SQLITE_FULL`, and
//! stay unknown.
//!
//! While the disk is full the writer keeps serving: reads never wait on it,
//! and each write fails on its own. The writer logs the condition once and
//! probes with backoff until a write commits again, then logs the recovery.
use super::*;
use std::cell::Cell;

/// The probe waits for this much free space on the database's volume before
/// it writes, so reused WAL space cannot report a recovery that the next
/// ordinary write undoes.
const PROBE_MIN_AVAILABLE_BYTES: u64 = 16 * 1024 * 1024;
/// The probe commits this many bytes (and frees them in the same transaction).
const PROBE_BYTES: i64 = 64 * 1024;
const PROBE_BASE_DELAY: Duration = Duration::from_millis(500);
const PROBE_MAX_DELAY: Duration = Duration::from_secs(15);

thread_local! {
    /// Set when a write failed because the disk is full. The writer owns its
    /// connection, so modules report it here rather than threading writer
    /// health through every request type; only the writer thread reads it.
    /// Shared error mapping may also set it on a query thread, where it is
    /// never read.
    static OBSERVED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn is_storage_full(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::DiskFull
    )
}

/// Classify a failed write on the writer thread. True means the disk is full
/// and, for a COMMIT, that the transaction did not commit.
pub(super) fn observe(error: &rusqlite::Error) -> bool {
    let full = is_storage_full(error);
    if full {
        OBSERVED.with(|observed| observed.set(true));
    }
    full
}

fn take_observed() -> bool {
    OBSERVED.with(|observed| observed.replace(false))
}

pub(super) fn initialize(connection: &Connection) -> Result<(), DaemonError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS durable_state_storage_probe (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                probed_at_ms INTEGER NOT NULL,
                payload BLOB
            );",
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.storage_probe",
            message: error.to_string(),
        })
}

/// What the owner should know about the durable writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DurableWriterCondition {
    Writable,
    /// The disk is full; writes fail until space is freed, then resume.
    StorageFull,
    /// A commit's outcome is unknown; only a restart recovers.
    Stopped,
}

impl DurableWriterCondition {
    fn code(self) -> u8 {
        match self {
            Self::Writable => 0,
            Self::StorageFull => 1,
            Self::Stopped => 2,
        }
    }
    /// The owner's notice when the writer enters this condition.
    pub(crate) fn notice(self) -> &'static str {
        match self {
            Self::Writable => "Kernel storage has space again. The kernel is saving changes again.",
            Self::StorageFull => {
                "Kernel storage is full. The kernel cannot save changes (App state, App installs \
                 and updates, logs) until disk space is freed on its volume; it then resumes on \
                 its own."
            }
            Self::Stopped => {
                "The kernel stopped saving changes after a save whose outcome it could not \
                 confirm. Restart the kernel to recover."
            }
        }
    }
}

impl DurableWriterHealth {
    pub(super) fn condition(&self) -> DurableWriterCondition {
        if self.stopped_uncertain.load(Ordering::Acquire) {
            DurableWriterCondition::Stopped
        } else if self.storage_full_since_ms.load(Ordering::Acquire) != 0 {
            DurableWriterCondition::StorageFull
        } else {
            DurableWriterCondition::Writable
        }
    }
    /// The condition when it changed since the last call, for a one-time
    /// owner notice per change.
    pub(super) fn take_condition_change(&self) -> Option<DurableWriterCondition> {
        let current = self.condition();
        let previous = self.announced.swap(current.code(), Ordering::AcqRel);
        (previous != current.code()).then_some(current)
    }
}

impl DurableKernelStateStore {
    #[cfg(test)]
    pub(crate) fn writer_condition(&self) -> DurableWriterCondition {
        self.writer.health.condition()
    }
    pub(crate) fn take_writer_condition_change(&self) -> Option<DurableWriterCondition> {
        self.writer.health.take_condition_change()
    }
}

/// The writer thread's view of a full disk: when to probe next, and how often
/// it has already failed.
pub(super) struct StorageProbe {
    database: Option<PathBuf>,
    attempt: u32,
    next: Option<Instant>,
}

impl StorageProbe {
    pub(super) fn new(connection: &Connection) -> Self {
        Self {
            database: connection
                .path()
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            attempt: 0,
            next: None,
        }
    }

    /// Called between writer requests: a request that hit a full disk puts
    /// the writer in the storage-full state.
    pub(super) fn after_request(&mut self, health: &DurableWriterHealth) {
        if take_observed() && self.next.is_none() {
            let now_ms = unix_ms().max(1);
            health
                .storage_full_since_ms
                .store(now_ms, Ordering::Release);
            self.attempt = 0;
            self.next = Some(Instant::now() + PROBE_BASE_DELAY);
            crate::logging::error_with_fields(
                "durable_state.storage",
                "kernel storage is full: durable writes fail until disk space is freed; the kernel resumes on its own",
                serde_json::json!({
                    "database": self.database.as_ref().map(|path| path.display().to_string()),
                    "available_bytes": self.available_bytes(),
                }),
            );
        }
    }

    /// The next request, probing for free space while the disk is full. A due
    /// probe runs before the next request, so a busy writer probes too.
    pub(super) fn next_request(
        &mut self,
        receiver: &Receiver<DurableWriterRequest>,
        connection: &mut Connection,
        health: &DurableWriterHealth,
    ) -> Option<DurableWriterRequest> {
        loop {
            self.after_request(health);
            // After a fence nothing commits, not even a probe: just wait for
            // the sender to go away.
            let Some(next) = self.next.filter(|_| !health.fatal.load(Ordering::Acquire)) else {
                return receiver.recv().ok();
            };
            let wait = next.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                self.probe(connection, health);
                continue;
            }
            match receiver.recv_timeout(wait) {
                Ok(request) => return Some(request),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn probe(&mut self, connection: &mut Connection, health: &DurableWriterHealth) {
        let available = self.available_bytes();
        let space = available.is_none_or(|bytes| bytes >= PROBE_MIN_AVAILABLE_BYTES);
        if space && probe_write(connection).is_ok() {
            let since = health.storage_full_since_ms.swap(0, Ordering::AcqRel);
            self.next = None;
            self.attempt = 0;
            // A write that failed while the probe ran is not a new episode.
            let _ = take_observed();
            crate::logging::warn_with_fields(
                "durable_state.storage",
                "kernel storage has space again: durable writes resumed",
                serde_json::json!({
                    "full_for_ms": unix_ms().saturating_sub(since),
                    "available_bytes": available,
                }),
            );
            return;
        }
        let _ = take_observed();
        self.attempt = self.attempt.saturating_add(1);
        self.next = Some(Instant::now() + probe_delay(self.attempt));
    }

    fn available_bytes(&self) -> Option<u64> {
        let directory = self.database.as_ref()?.parent()?;
        fs2::available_space(directory).ok()
    }
}

fn probe_delay(attempt: u32) -> Duration {
    PROBE_BASE_DELAY
        .saturating_mul(1_u32 << attempt.min(8))
        .min(PROBE_MAX_DELAY)
}

/// Commit a real allocation: write `PROBE_BYTES`, then free them, in one
/// FULL-synchronous transaction. The freed pages stay on the free list, so
/// probes do not grow the database.
fn probe_write(connection: &mut Connection) -> rusqlite::Result<()> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO durable_state_storage_probe (id, probed_at_ms, payload)
         VALUES (1, ?1, zeroblob(?2))
         ON CONFLICT(id) DO UPDATE SET probed_at_ms = excluded.probed_at_ms,
             payload = excluded.payload",
        params![unix_ms() as i64, PROBE_BYTES],
    )?;
    tx.execute(
        "UPDATE durable_state_storage_probe SET payload = NULL WHERE id = 1",
        [],
    )?;
    tx.commit()
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod tests;
