//! A terminal's PTY, parser and readers share only this terminal's lock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use tokio::sync::{oneshot, Notify};

use super::terminal::Terminal;
use super::types::RoomTerminalEvent;
use crate::error::DaemonError;
use crate::pty::PtyManager;

pub(super) struct TerminalOwner {
    pub session_id: String,
    pub title: String,
    exited: AtomicBool,
    state: Mutex<TerminalState>,
    pub output_changed: Notify,
    pub pump_wake: Notify,
}

pub(super) struct TerminalState {
    pub terminal: Terminal,
    pub pty: PtyManager,
    pub waiters: BTreeMap<String, oneshot::Sender<RoomTerminalEvent>>,
    pub next_wait: u64,
    pub active_until: Instant,
    pub removed: bool,
    pub shutdown: bool,
    /// A hot reload holds this terminal's output for the next kernel program.
    pub handed_over: bool,
}

impl TerminalOwner {
    pub fn new(terminal: Terminal, pty: PtyManager) -> Self {
        let exited = !terminal.is_running();
        Self {
            session_id: terminal.record.session_id.clone(),
            title: terminal.record.title.clone(),
            exited: AtomicBool::new(exited),
            state: Mutex::new(TerminalState {
                terminal,
                pty,
                waiters: BTreeMap::new(),
                next_wait: 0,
                active_until: Instant::now(),
                removed: false,
                shutdown: false,
                handed_over: false,
            }),
            output_changed: Notify::new(),
            pump_wake: Notify::new(),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, TerminalState> {
        self.state.lock().expect("room terminal state poisoned")
    }

    pub fn lock_open(&self) -> Result<MutexGuard<'_, TerminalState>, DaemonError> {
        let state = self.lock();
        if state.removed {
            return Err(super::service::refused(format!(
                "terminal `{}` was closed",
                state.terminal.record.terminal_id
            )));
        }
        Ok(state)
    }

    /// A busy parser is conservatively work. Checking a managed stop must
    /// never wait for another terminal's parsing or rendering.
    pub fn has_running_work(&self) -> bool {
        // A PTY never restarts in the same owner. Later rendering or persistence
        // locks must not turn its known exit into a new busy/idle activity cycle.
        if self.exited.load(Ordering::Acquire) {
            return false;
        }
        let state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => return true,
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("room terminal state poisoned"),
        };
        let terminal = &state.terminal;
        if state.removed || !terminal.is_running() {
            self.exited.store(true, Ordering::Release);
            return false;
        }
        if terminal.is_busy(&state.pty) || terminal.driver.launching(Instant::now()) {
            return true;
        }
        let sweep = terminal.session_sweep;
        drop(state);
        sweep.is_some_and(|sweep| sweep.has_jobs())
    }
}
