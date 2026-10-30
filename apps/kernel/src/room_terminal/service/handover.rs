//! Running terminals across a hot reload: the old kernel program holds their
//! output and hands them over; the new program adopts them by terminal id.

use serde::{Deserialize, Serialize};

use super::*;
use crate::pty::PtyHandover;

/// How long a terminal's reader may take to stop between two reads.
const HOLD_TIMEOUT: Duration = Duration::from_secs(2);

/// One running terminal as the next kernel program receives it. The screen
/// comes from the terminal's saved record; this carries what lived in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHandover {
    pub(crate) terminal_id: String,
    pub(crate) pty: PtyHandover,
    /// The absolute offset of the first retained output byte.
    pub(crate) output_start: u64,
    pub(crate) output_base64: String,
    pub(crate) mark_tracking: bool,
    pub(crate) command_in_flight: bool,
    pub(crate) program_running: bool,
    pub(crate) commands_started: u64,
}

impl TerminalHandover {
    pub(crate) fn master_fd(&self) -> i32 {
        self.pty.master_fd
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pty.pid
    }

    pub(crate) fn retained_output(&self) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.output_base64)
            .unwrap_or_default()
    }
}

impl RoomTerminalService {
    /// Holds every running terminal's output between two reads, applies what
    /// was already read and saves its record. After a failed swap,
    /// `release_handover` lets them run on in this program.
    pub fn hand_over(&self) -> Result<Vec<TerminalHandover>, DaemonError> {
        let mut handovers = Vec::new();
        for owner in self.owners() {
            let mut state = owner.lock();
            if state.removed || state.shutdown {
                continue;
            }
            state.handed_over = true;
            let TerminalState { terminal, pty, .. } = &mut *state;
            let Some(run_id) = terminal.run_id.clone() else {
                continue;
            };
            let deadline = Instant::now() + HOLD_TIMEOUT;
            // A full output queue keeps the reader from its stopping point: drain it.
            while !pty.hold_output(&run_id, Duration::from_millis(200))? {
                terminal.pump_output(pty, Instant::now());
                if Instant::now() >= deadline {
                    return Err(refused(format!(
                        "terminal `{}` did not pause its output",
                        terminal.record.terminal_id
                    )));
                }
            }
            terminal.pump_output(pty, Instant::now());
            handovers.push(terminal.handover(pty.handover(&run_id)?));
            let record = terminal.persisted_record();
            self.inner.store.enqueue_save(&record)?.wait()?;
        }
        Ok(handovers)
    }

    pub fn release_handover(&self) {
        for owner in self.owners() {
            let mut state = owner.lock();
            state.handed_over = false;
            if let Some(run_id) = state.terminal.run_id.clone() {
                state.pty.release_output(&run_id);
            }
        }
    }

    /// Adopts the terminals a hot reload handed over; any that cannot be
    /// adopted ends as stopped by the restart, as without a handover, and is
    /// returned to be saved so.
    pub(super) fn adopt(
        records: Vec<TerminalRecord>,
        mut handovers: BTreeMap<String, TerminalHandover>,
    ) -> (BTreeMap<String, Arc<TerminalOwner>>, Vec<TerminalRecord>) {
        let mut owners = BTreeMap::new();
        let mut unadopted = Vec::new();
        for record in records {
            let terminal_id = record.terminal_id.clone();
            let owner = match handovers.remove(&terminal_id) {
                Some(handover) => match adopt_one(record.clone(), &handover) {
                    Ok(owner) => owner,
                    Err(error) => {
                        let mut record = record;
                        record.ended = Some(RoomTerminalOutcome::KernelRestarted);
                        crate::logging::error_with_fields(
                            "room_terminal.hot_reload",
                            "a handed-over terminal could not be adopted",
                            serde_json::json!({
                                "terminal_id": terminal_id,
                                "error": error.to_string(),
                            }),
                        );
                        unadopted.push(record.clone());
                        TerminalOwner::new(Terminal::recovered(record), PtyManager::new())
                    }
                },
                None => TerminalOwner::new(Terminal::recovered(record), PtyManager::new()),
            };
            owners.insert(terminal_id, Arc::new(owner));
        }
        // A handover without a record has nobody to show it: close its PTY.
        for handover in handovers.into_values() {
            unsafe { libc::close(handover.pty.master_fd) };
        }
        (owners, unadopted)
    }
}

fn adopt_one(
    record: TerminalRecord,
    handover: &TerminalHandover,
) -> Result<TerminalOwner, DaemonError> {
    let mut pty = PtyManager::new();
    let terminal = Terminal::adopted(record, handover);
    let run_id = terminal.run_id.clone().unwrap_or_default();
    pty.adopt(run_id, &handover.pty)?;
    Ok(TerminalOwner::new(terminal, pty))
}
