//! Room-scoped terminals on the kernel's `PtyManager`.
//!
//! The service is authority-free: callers have already decided who may
//! reach a Room's terminals and which actor a request speaks for.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore;
use tokio::sync::{broadcast, Notify};

use super::driver::Admission;
use super::store::{TerminalRecord, TerminalStore};
use super::terminal::Terminal;
use super::types::{
    RoomTerminalDriver, RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalInputResult,
    RoomTerminalOutcome, RoomTerminalOutput, RoomTerminalSnapshot, RoomTerminalSummary,
    RoomTerminalWatch, TerminalActor,
};
use super::watch::{Fired, QUIET_BEFORE_WAITING};
use crate::error::DaemonError;
use crate::pty::{PtyManager, PtyProcessState, PtySpawnRequest};
use crate::session::unix_epoch_ms;

const MAX_TERMINALS_PER_ROOM: usize = 16;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_PAGE_BYTES: usize = 256 * 1024;
const PUMP_INTERVAL: Duration = Duration::from_millis(100);
const PERSIST_INTERVAL: Duration = Duration::from_secs(1);

pub struct OpenRoomTerminal {
    pub session_id: String,
    pub working_directory: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub title: Option<String>,
    pub opened_by: TerminalActor,
}

#[derive(Clone)]
pub struct RoomTerminalService {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    store: TerminalStore,
    events: broadcast::Sender<RoomTerminalEvent>,
    output_changed: Notify,
    pump_started: AtomicBool,
}

struct State {
    pty: PtyManager,
    terminals: BTreeMap<String, Terminal>,
}

impl RoomTerminalService {
    /// Terminals that ran when the kernel last stopped come back as ended by
    /// the restart, scrollback kept.
    pub fn new(store_root: PathBuf) -> Self {
        let store = TerminalStore::new(store_root);
        let terminals = match store.recover() {
            Ok(records) => records
                .into_iter()
                .map(|record| (record.terminal_id.clone(), Terminal::recovered(record)))
                .collect(),
            Err(error) => {
                crate::logging::error_with_fields(
                    "room_terminal.recover",
                    "room terminals could not be restored",
                    serde_json::json!({ "error": error.to_string() }),
                );
                BTreeMap::new()
            }
        };
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    pty: PtyManager::new(),
                    terminals,
                }),
                store,
                events: broadcast::channel(256).0,
                output_changed: Notify::new(),
                pump_started: AtomicBool::new(false),
            }),
        }
    }

    pub fn open(&self, spec: OpenRoomTerminal) -> Result<RoomTerminalSummary, DaemonError> {
        validate_size(spec.cols, spec.rows)?;
        if !spec.working_directory.is_dir() {
            return Err(refused(format!(
                "the Room's project folder `{}` does not exist",
                spec.working_directory.display()
            )));
        }
        let shell = login_shell()?;
        let terminal_id = random_terminal_id();
        let run_id = format!("room-terminal:{terminal_id}");
        let mut state = self.lock();
        let in_room: Vec<&Terminal> = state
            .terminals
            .values()
            .filter(|terminal| terminal.record.session_id == spec.session_id)
            .collect();
        if in_room.len() >= MAX_TERMINALS_PER_ROOM {
            return Err(refused(format!(
                "this Room already has {MAX_TERMINALS_PER_ROOM} terminals; close one first"
            )));
        }
        let title = spec
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| next_title(&in_room));
        state.pty.spawn_interactive(PtySpawnRequest {
            process_key: run_id.clone(),
            provider_run_id: run_id.clone(),
            program: shell.clone(),
            args: vec!["-l".to_string(), "-i".to_string()],
            env: BTreeMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
            env_remove: Vec::new(),
            working_directory: Some(spec.working_directory.clone()),
            cols: spec.cols,
            rows: spec.rows,
        })?;
        let shell_pid = state
            .pty
            .process_id(&run_id)
            .ok()
            .flatten()
            .and_then(|pid| i32::try_from(pid).ok());
        let record = TerminalRecord {
            terminal_id: terminal_id.clone(),
            session_id: spec.session_id,
            title,
            program: shell,
            working_directory: spec.working_directory.display().to_string(),
            cols: spec.cols,
            rows: spec.rows,
            opened_by: spec.opened_by,
            created_at_ms: unix_epoch_ms(),
            ended: None,
            scrollback_text: String::new(),
            screen_text: String::new(),
        };
        if let Err(error) = self.inner.store.save(&record) {
            let _ = state.pty.remove_process(&run_id);
            return Err(error);
        }
        let terminal = Terminal::running(record, run_id, shell_pid);
        let summary = terminal.summary(RoomTerminalDriver::Free);
        state.terminals.insert(terminal_id, terminal);
        drop(state);
        self.start_pump();
        Ok(summary)
    }

    pub fn list(&self, session_id: &str) -> Vec<RoomTerminalSummary> {
        let mut state = self.lock();
        let State { pty, terminals } = &mut *state;
        let mut summaries: Vec<_> = terminals
            .values_mut()
            .filter(|terminal| terminal.record.session_id == session_id)
            .map(|terminal| {
                let driver = current_driver(pty, terminal);
                terminal.summary(driver)
            })
            .collect();
        summaries.sort_by_key(|summary| summary.created_at_ms);
        summaries
    }

    pub fn snapshot(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<RoomTerminalSnapshot, DaemonError> {
        let mut state = self.lock();
        let State { pty, terminals } = &mut *state;
        let terminal = find(terminals, session_id, terminal_id)?;
        let driver = current_driver(pty, terminal);
        Ok(terminal.snapshot(driver))
    }

    /// Write `input` for `actor`, as one unit, unless the one-driver rule
    /// refuses it.
    pub async fn input(
        &self,
        session_id: &str,
        terminal_id: &str,
        actor: &TerminalActor,
        input: &[u8],
    ) -> Result<RoomTerminalInputResult, DaemonError> {
        if input.is_empty() || input.len() > MAX_INPUT_BYTES {
            return Err(refused(format!(
                "terminal input must be 1 to {MAX_INPUT_BYTES} bytes"
            )));
        }
        loop {
            {
                let mut state = self.lock();
                let State { pty, terminals } = &mut *state;
                let terminal = find(terminals, session_id, terminal_id)?;
                let Some(run_id) = terminal.run_id.clone() else {
                    return Err(ended_error(terminal));
                };
                let shell_foreground = shell_is_foreground(pty, terminal);
                let now = Instant::now();
                match terminal.driver.admit(actor, shell_foreground, now) {
                    Admission::Admit => {
                        pty.write_input(&run_id, input)?;
                        terminal
                            .driver
                            .record_write(actor, shell_foreground, input, now);
                        let driver = terminal.driver.observe(shell_foreground);
                        return Ok(RoomTerminalInputResult {
                            accepted: true,
                            driver,
                            refusal: None,
                        });
                    }
                    Admission::Refuse(driver) => {
                        let refusal = match &driver {
                            RoomTerminalDriver::Busy { driven_by } => {
                                format!("busy (driven by {driven_by})")
                            }
                            RoomTerminalDriver::Free => "busy".to_string(),
                        };
                        return Ok(RoomTerminalInputResult {
                            accepted: false,
                            driver,
                            refusal: Some(refusal),
                        });
                    }
                    Admission::Settle => {}
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn resize(
        &self,
        session_id: &str,
        terminal_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<RoomTerminalSummary, DaemonError> {
        validate_size(cols, rows)?;
        let mut state = self.lock();
        let State { pty, terminals } = &mut *state;
        let terminal = find(terminals, session_id, terminal_id)?;
        let Some(run_id) = terminal.run_id.clone() else {
            return Err(ended_error(terminal));
        };
        pty.resize(&run_id, cols, rows)?;
        terminal.resize(cols, rows);
        let driver = current_driver(pty, terminal);
        Ok(terminal.summary(driver))
    }

    /// Output after `after_offset`, waiting up to `wait` for some.
    pub async fn read_output(
        &self,
        session_id: &str,
        terminal_id: &str,
        after_offset: u64,
        wait: Duration,
    ) -> Result<RoomTerminalOutput, DaemonError> {
        let deadline = Instant::now() + wait;
        loop {
            let changed = self.inner.output_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.lock();
                let State { pty, terminals } = &mut *state;
                let terminal = find(terminals, session_id, terminal_id)?;
                let page = terminal.log.read(after_offset, MAX_OUTPUT_PAGE_BYTES);
                let finished = !terminal.is_running() || Instant::now() >= deadline;
                if !page.bytes.is_empty() || page.gap || finished {
                    let driver = current_driver(pty, terminal);
                    return Ok(RoomTerminalOutput {
                        terminal: terminal.summary(driver),
                        data_base64: base64::engine::general_purpose::STANDARD.encode(&page.bytes),
                        next_offset: page.next_offset,
                        gap: page.gap,
                    });
                }
            }
            let _ = tokio::time::timeout_at(deadline.into(), changed).await;
        }
    }

    pub fn close(&self, session_id: &str, terminal_id: &str) -> Result<(), DaemonError> {
        let mut state = self.lock();
        find(&mut state.terminals, session_id, terminal_id)?;
        self.remove(&mut state, terminal_id);
        Ok(())
    }

    /// The Room ended: its terminals end with it.
    pub fn end_session(&self, session_id: &str) {
        let mut state = self.lock();
        let ids: Vec<String> = state
            .terminals
            .values()
            .filter(|terminal| terminal.record.session_id == session_id)
            .map(|terminal| terminal.record.terminal_id.clone())
            .collect();
        for id in ids {
            self.remove(&mut state, &id);
        }
    }

    /// Ask to be told when the terminal finishes a program, prints a text or
    /// waits for input. `armed_by` is the agent to wake.
    pub fn watch(
        &self,
        session_id: &str,
        terminal_id: &str,
        armed_by: &str,
        watch: RoomTerminalWatch,
    ) -> Result<(), DaemonError> {
        let mut state = self.lock();
        let State { pty, terminals } = &mut *state;
        let terminal = find(terminals, session_id, terminal_id)?;
        if !terminal.is_running() {
            return Err(ended_error(terminal));
        }
        let program_running = !shell_is_foreground(pty, terminal);
        terminal
            .watches
            .arm(armed_by.to_string(), watch, &terminal.plain, program_running);
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoomTerminalEvent> {
        self.inner.events.subscribe()
    }

    fn remove(&self, state: &mut State, terminal_id: &str) {
        if let Some(run_id) = state
            .terminals
            .remove(terminal_id)
            .and_then(|terminal| terminal.run_id)
        {
            if let Err(error) = state.pty.remove_process(&run_id) {
                crate::logging::error_with_fields(
                    "room_terminal.close",
                    "terminal process could not be removed",
                    serde_json::json!({ "terminal_id": terminal_id, "error": error.to_string() }),
                );
            }
        }
        self.inner.store.remove(terminal_id);
    }

    fn start_pump(&self) {
        if self.inner.pump_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                service.pump_once();
                tokio::time::sleep(PUMP_INTERVAL).await;
            }
        });
    }

    /// Move PTY output into screens, end terminals whose shell exited, track
    /// the driver, fire watches and persist what changed.
    pub(super) fn pump_once(&self) {
        let mut fired: Vec<RoomTerminalEvent> = Vec::new();
        let mut output_changed = false;
        {
            let mut state = self.lock();
            let State { pty, terminals } = &mut *state;
            let now = Instant::now();
            for terminal in terminals.values_mut() {
                let Some(run_id) = terminal.run_id.clone() else {
                    continue;
                };
                let mut produced = false;
                let mut drain = |pty: &mut PtyManager, terminal: &mut Terminal| {
                    for chunk in pty.drain_output(&run_id).unwrap_or_default() {
                        terminal.apply_output(&chunk.bytes, now);
                        produced = true;
                    }
                };
                drain(pty, terminal);
                let mut watch_events = Fired::new();
                if produced {
                    watch_events.extend(terminal.watches.on_output(&terminal.plain));
                }
                if let Ok(PtyProcessState::Exited { exit_code, signal }) =
                    pty.poll_process_state(&run_id)
                {
                    drain(pty, terminal);
                    if produced {
                        watch_events.extend(terminal.watches.on_output(&terminal.plain));
                    }
                    let _ = pty.remove_process(&run_id);
                    watch_events.extend(
                        terminal
                            .watches
                            .on_shell_exit(exit_code, signal.clone()),
                    );
                    terminal.end(RoomTerminalOutcome::ShellExited { exit_code, signal });
                } else {
                    let program_running = !shell_is_foreground(pty, terminal);
                    terminal.program_running = program_running;
                    watch_events.extend(terminal.watches.on_foreground(program_running));
                    if program_running
                        && !terminal.waiting_reported
                        && now.duration_since(terminal.last_output_at) >= QUIET_BEFORE_WAITING
                        && terminal.watches.is_waiting_for_input()
                    {
                        terminal.waiting_reported = true;
                        watch_events.extend(terminal.watches.on_quiet_program());
                    }
                }
                output_changed |= produced || !terminal.is_running();
                if !watch_events.is_empty() {
                    let screen_text = terminal.screen.visible_text();
                    fired.extend(watch_events.into_iter().map(|(armed_by, kind)| {
                        event(terminal, armed_by, kind, screen_text.clone())
                    }));
                }
                let due = terminal.unsaved_since.is_some_and(|since| {
                    !terminal.is_running() || now.duration_since(since) >= PERSIST_INTERVAL
                });
                if due {
                    terminal.unsaved_since = None;
                    if let Err(error) = self.inner.store.save(&terminal.to_record()) {
                        crate::logging::error_with_fields(
                            "room_terminal.persist",
                            "terminal scrollback could not be saved",
                            serde_json::json!({ "error": error.to_string() }),
                        );
                    }
                }
            }
        }
        if output_changed {
            self.inner.output_changed.notify_waiters();
        }
        for event in fired {
            let _ = self.inner.events.send(event);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().expect("room terminal state poisoned")
    }
}

fn event(
    terminal: &Terminal,
    armed_by: String,
    kind: RoomTerminalEventKind,
    screen_text: String,
) -> RoomTerminalEvent {
    RoomTerminalEvent {
        terminal_id: terminal.record.terminal_id.clone(),
        session_id: terminal.record.session_id.clone(),
        armed_by,
        kind,
        screen_text,
    }
}

fn find<'a>(
    terminals: &'a mut BTreeMap<String, Terminal>,
    session_id: &str,
    terminal_id: &str,
) -> Result<&'a mut Terminal, DaemonError> {
    terminals
        .get_mut(terminal_id)
        .filter(|terminal| terminal.record.session_id == session_id)
        .ok_or_else(|| refused(format!("this Room has no terminal `{terminal_id}`")))
}

fn shell_is_foreground(pty: &PtyManager, terminal: &Terminal) -> bool {
    match (&terminal.run_id, terminal.shell_pid) {
        (Some(run_id), Some(shell_pid)) => {
            pty.foreground_process_group(run_id).ok().flatten() == Some(shell_pid)
        }
        _ => true,
    }
}

fn current_driver(pty: &PtyManager, terminal: &mut Terminal) -> RoomTerminalDriver {
    let shell_foreground = shell_is_foreground(pty, terminal);
    terminal.driver.observe(shell_foreground)
}

fn validate_size(cols: u16, rows: u16) -> Result<(), DaemonError> {
    if (1..=500).contains(&cols) && (1..=200).contains(&rows) {
        Ok(())
    } else {
        Err(refused(
            "terminal size must be 1-500 columns by 1-200 rows".to_string(),
        ))
    }
}

/// The user's own shell, as Claude Code and Codex use it.
fn login_shell() -> Result<String, DaemonError> {
    match std::env::var("SHELL") {
        Ok(shell) if std::path::Path::new(&shell).is_absolute() => Ok(shell),
        _ => Err(refused(
            "$SHELL is not an absolute path in this kernel's environment; start the kernel from your login environment or set SHELL"
                .to_string(),
        )),
    }
}

fn next_title(in_room: &[&Terminal]) -> String {
    (1..)
        .map(|number| format!("Terminal {number}"))
        .find(|title| in_room.iter().all(|terminal| terminal.record.title != *title))
        .expect("an unused title exists")
}

fn random_terminal_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("term-{suffix}")
}

fn ended_error(terminal: &Terminal) -> DaemonError {
    let description = terminal
        .record
        .ended
        .as_ref()
        .map(RoomTerminalOutcome::description)
        .unwrap_or_else(|| "ended".to_string());
    refused(format!(
        "terminal `{}` has {description}; open a new terminal",
        terminal.record.terminal_id
    ))
}

fn refused(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room terminal",
        message,
    }
}
