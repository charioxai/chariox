//! MP-09 G05 / G18 drill hooks. `CHARIOX_AGENT_FAULT_DIR` names a lane-owned
//! directory, read when the kernel opens its durable state (the pattern of
//! `CHARIOX_WAKE_SCHEDULER_STALL_FILE`). While `writer-fail` exists in it, every
//! agent lifecycle write fails before commit, as a failed disk commit does.
//! While `duplicate-delivery` exists, each committed delivery of one occurrence
//! (source or wake occurrence, message, wake firing, completion, owner response,
//! delivery receipt) is applied a second time: the ledger must keep it one
//! logical delivery.
//! A `drop-ack` file holding a delivery kind (the names of `delivery_kind`)
//! drops one acknowledgement: the next write of that kind commits, the caller is
//! told it failed (as when the reply is lost after the commit), and the file is
//! removed, so the retry meets the committed delivery. `drop-ack.log` records it.
//! Without the variable nothing changes.
use super::Operation;
use std::path::PathBuf;

const FAULT_DIR_ENV: &str = "CHARIOX_AGENT_FAULT_DIR";

#[derive(Default)]
pub(crate) struct AgentFaults {
    dir: Option<PathBuf>,
}

#[cfg(test)]
thread_local! {
    static TEST_FAULT_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Tests: stores opened on this thread while the guard lives use `dir`, so a
/// fault never reaches another test's store.
#[cfg(test)]
pub(crate) struct TestFaultDir;
#[cfg(test)]
impl TestFaultDir {
    pub(crate) fn set(dir: &std::path::Path) -> Self {
        TEST_FAULT_DIR.with(|d| *d.borrow_mut() = Some(dir.to_path_buf()));
        Self
    }
}
#[cfg(test)]
impl Drop for TestFaultDir {
    fn drop(&mut self) {
        TEST_FAULT_DIR.with(|d| *d.borrow_mut() = None);
    }
}

impl AgentFaults {
    /// Read on the thread that opens the store, before its writer starts.
    pub(crate) fn from_env() -> Self {
        #[cfg(test)]
        if let Some(dir) = TEST_FAULT_DIR.with(|d| d.borrow().clone()) {
            return Self { dir: Some(dir) };
        }
        Self {
            dir: std::env::var_os(FAULT_DIR_ENV).map(PathBuf::from),
        }
    }
    pub(super) fn writer_fails(&self) -> bool {
        self.active("writer-fail")
    }
    pub(super) fn duplicates_deliveries(&self) -> bool {
        self.active("duplicate-delivery")
    }
    /// The `drop-ack` file names this operation's delivery kind.
    pub(super) fn wants_ack_dropped(&self, operation: &Operation) -> bool {
        let (Some(dir), Some(kind)) = (&self.dir, delivery_kind(operation)) else {
            return false;
        };
        std::fs::read_to_string(dir.join("drop-ack")).is_ok_and(|text| text.trim() == kind)
    }
    /// After the commit: the fault fires once, so the retry meets the committed write.
    pub(super) fn ack_dropped(&self, operation_kind: &str) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join("drop-ack"));
        }
        self.record_line(
            "drop-ack.log",
            &format!("{operation_kind} acknowledgement dropped"),
        );
    }
    /// One line per redelivery in `duplicate-delivery.log`, so a drill can
    /// count what was replayed next to the effects it observes.
    pub(super) fn record_duplicate(&self, kind: &str, error: Option<&crate::error::DaemonError>) {
        self.record_line(
            "duplicate-delivery.log",
            &format!(
                "{kind} {}",
                error.map_or_else(|| "applied".to_string(), |e| format!("refused: {e}"))
            ),
        );
    }
    fn record_line(&self, log: &str, text: &str) {
        let Some(dir) = &self.dir else { return };
        let line = format!("{} {text}\n", crate::session::unix_epoch_ms());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(log))
            .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
    }
    fn active(&self, name: &str) -> bool {
        self.dir.as_ref().is_some_and(|dir| dir.join(name).exists())
    }
}

pub(super) fn delivery_kind(operation: &Operation) -> Option<&'static str> {
    match operation {
        Operation::Occur(_) => Some("occurrence"),
        Operation::Send { .. } => Some("message"),
        Operation::FireWakes { .. } => Some("wake_intent"),
        Operation::WakeOccurred { .. } => Some("wake_occurrence"),
        Operation::ProcessMatched { .. } | Operation::ProcessExited { .. } => {
            Some("process_occurrence")
        }
        Operation::SourceOutcome { .. } | Operation::DelegateUnavailable { .. } => {
            Some("completion")
        }
        Operation::OwnerResponse { .. } => Some("owner_response"),
        Operation::Receipt { .. } | Operation::StartFailed { .. } => Some("delivery_receipt"),
        _ => None,
    }
}
