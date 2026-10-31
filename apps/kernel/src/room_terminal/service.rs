//! Room-scoped terminals on the kernel's `PtyManager`.
//!
//! The service is authority-free: callers have already decided who may
//! reach a Room's terminals and which actor a request speaks for.

use super::REFUSED_CODE;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore;
use tokio::sync::oneshot;

use super::driver::Admission;
use super::events::{BoundEvents, CompletionEvents, WatchOwner};
use super::managed_work::ManagedWork;
use super::owner::{TerminalOwner, TerminalState};
use super::shell::login_shell;
use super::shell_marks::ShellIntegration;
use super::store::{TerminalRecord, TerminalStore};
use super::terminal::Terminal;
use super::types::{
    RoomTerminalDriver, RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalInputResult,
    RoomTerminalOutcome, RoomTerminalOutput, RoomTerminalSnapshot, RoomTerminalSummary,
    RoomTerminalWaitResult, RoomTerminalWatch, TerminalActor,
};
use crate::error::DaemonError;
use crate::pty::{PtyManager, PtySpawnRequest};
use crate::runtime::state::ManagedKernelQuiescenceGate;
use crate::session::unix_epoch_ms;

mod outside_events;
mod pump_task;

const MAX_TERMINALS_PER_ROOM: usize = 16;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_PAGE_BYTES: usize = 256 * 1024;
const PUMP_INTERVAL: Duration = Duration::from_millis(100);
/// While a person types or output flows, look at the PTY this often so an
/// echo does not wait for a tick that only exists to notice quiet changes.
const ACTIVE_PUMP_INTERVAL: Duration = Duration::from_millis(5);
const ACTIVE_PUMP_WINDOW: Duration = Duration::from_millis(500);
const PERSIST_INTERVAL: Duration = Duration::from_secs(1);
/// Below the provider tool limit (Codex gives a tool call 300 s).
pub const MAX_WAIT: Duration = Duration::from_secs(240);

pub(super) fn output_wait(wait: Duration) -> Duration {
    wait.min(MAX_WAIT)
}

/// Kernel-private tokens and configs the user's shell must not inherit.
pub(crate) const KERNEL_SECRET_ENVIRONMENT: &[&str] = &[
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    "CHARIOX_CLOUD_RELAY_CONFIG_PATH",
];

pub struct OpenRoomTerminal {
    pub session_id: String,
    pub working_directory: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub title: Option<String>,
    pub opened_by: TerminalActor,
    /// Run this program (argv, no shell) instead of a login shell.
    pub command: Option<Vec<String>>,
}

#[derive(Clone)]
pub struct RoomTerminalService {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    /// Serialize terminal creation with Room end and kernel shutdown.
    lifecycle: Mutex<()>,
    store: TerminalStore,
    events: BoundEvents,
    outside_events: std::sync::RwLock<Option<crate::durable_state::DurableKernelStateStore>>,
    shell_integration: ShellIntegration,
    managed: ManagedWork,
    pump_runtime: OnceLock<tokio::runtime::Handle>,
}

pub(super) struct State {
    pub(super) terminals: BTreeMap<String, Arc<TerminalOwner>>,
    pub(super) shutdown: bool,
}

impl RoomTerminalService {
    pub(crate) fn process_keys(
        &self,
        session_id: &str,
    ) -> Vec<(
        crate::runtime::process_resources::SessionProcessOwner,
        String,
    )> {
        let owners: Vec<_> = self
            .lock()
            .terminals
            .values()
            .filter(|owner| owner.session_id == session_id)
            .cloned()
            .collect();
        owners
            .iter()
            .filter_map(|owner| {
                let state = owner.lock();
                state.terminal.run_id.as_ref().map(|run| {
                    (
                        crate::runtime::process_resources::SessionProcessOwner::Terminal(
                            state.terminal.record.terminal_id.clone(),
                        ),
                        run.clone(),
                    )
                })
            })
            .collect()
    }
    /// Terminals that ran when the kernel last stopped come back as ended by
    /// the restart, scrollback kept.
    pub fn new(store_root: PathBuf) -> Result<Self, DaemonError> {
        let shell_integration = ShellIntegration::install(&store_root.with_extension("shell"));
        let store = TerminalStore::new(store_root)?;
        let terminals = store
            .recover()?
            .into_iter()
            .map(|record| {
                (
                    record.terminal_id.clone(),
                    Arc::new(TerminalOwner::new(
                        Terminal::recovered(record),
                        PtyManager::new(),
                    )),
                )
            })
            .collect();
        Ok(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    terminals,
                    shutdown: false,
                }),
                lifecycle: Mutex::new(()),
                store,
                events: BoundEvents::default(),
                outside_events: std::sync::RwLock::new(None),
                shell_integration,
                managed: ManagedWork::default(),
                pump_runtime: OnceLock::new(),
            }),
        })
    }

    /// Where fired watches go. Bound by the event core, which owns them: the service holds the
    /// sink weakly so the sink may hold the runtime that owns this service.
    pub fn bind_events(&self, events: Weak<dyn CompletionEvents>) {
        self.inner.events.bind(events);
    }

    pub fn open(&self, spec: OpenRoomTerminal) -> Result<RoomTerminalSummary, DaemonError> {
        validate_size(spec.cols, spec.rows)?;
        if !spec.working_directory.is_dir() {
            return Err(refused(format!(
                "the Room's project folder `{}` does not exist",
                spec.working_directory.display()
            )));
        }
        let (program, args) = match &spec.command {
            None => {
                let shell = login_shell()?;
                let args = self.inner.shell_integration.arguments(&shell);
                (shell, args)
            }
            Some(argv) => {
                let valid = !argv.is_empty()
                    && argv.len() <= 64
                    && !argv[0].is_empty()
                    && argv
                        .iter()
                        .all(|arg| arg.len() <= 4096 && !arg.contains('\0'));
                if !valid {
                    return Err(refused("command must be 1-64 arguments".to_string()));
                }
                (argv[0].clone(), argv[1..].to_vec())
            }
        };
        // A program started now is work: it is admitted only while a managed
        // kernel is not being stopped. A shell alone is not work.
        let _admission = match spec.command {
            Some(_) => self
                .inner
                .managed
                .admission()
                .map_err(|error| refuse_fenced(&error))?,
            None => None,
        };
        let creation = self
            .inner
            .lifecycle
            .lock()
            .expect("terminal lifecycle poisoned");
        let title = {
            let state = self.lock();
            if state.shutdown {
                return Err(refused(
                    "this kernel is stopping, so it opens no terminals".to_string(),
                ));
            }
            let in_room: Vec<_> = state
                .terminals
                .values()
                .filter(|owner| owner.session_id == spec.session_id)
                .collect();
            if in_room.len() >= MAX_TERMINALS_PER_ROOM {
                return Err(refused(format!(
                    "this Room already has {MAX_TERMINALS_PER_ROOM} terminals; close one first"
                )));
            }
            spec.title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| next_title(&in_room))
        };
        let user_interacted = spec.opened_by == TerminalActor::User;
        let terminal_id = random_terminal_id();
        let run_id = format!("room-terminal:{terminal_id}");
        let mut pty = PtyManager::new();
        let mut env = BTreeMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        let mut reports_first_prompt = false;
        if spec.command.is_none() {
            let marks = self.inner.shell_integration.environment(&program);
            reports_first_prompt = !marks.is_empty();
            env.extend(marks);
        }
        pty.spawn_interactive(PtySpawnRequest {
            process_key: run_id.clone(),
            provider_run_id: run_id.clone(),
            program: program.clone(),
            args,
            env,
            env_remove: KERNEL_SECRET_ENVIRONMENT
                .iter()
                .map(|name| name.to_string())
                .collect(),
            working_directory: Some(spec.working_directory.clone()),
            cols: spec.cols,
            rows: spec.rows,
        })?;
        let shell_pid = pty
            .process_id(&run_id)
            .ok()
            .flatten()
            .and_then(|pid| i32::try_from(pid).ok());
        let record = TerminalRecord {
            terminal_id: terminal_id.clone(),
            session_id: spec.session_id,
            title,
            program,
            working_directory: spec.working_directory.display().to_string(),
            cols: spec.cols,
            rows: spec.rows,
            opened_by: spec.opened_by,
            command: spec.command,
            user_interacted,
            created_at_ms: unix_epoch_ms(),
            ended: None,
            scrollback_text: String::new(),
            screen_text: String::new(),
        };
        let terminal = Terminal::running(record, run_id.clone(), shell_pid, reports_first_prompt);
        let (owner, pending, summary) = {
            let mut state = self.lock();
            let pending = match self.inner.store.enqueue_save(&terminal.record) {
                Ok(pending) => pending,
                Err(error) => {
                    drop(state);
                    if let Some(sweep) = &terminal.session_sweep {
                        sweep.stop();
                    }
                    pty.remove_process(&run_id)?;
                    return Err(error);
                }
            };
            let summary = terminal.summary(RoomTerminalDriver::Free);
            let owner = Arc::new(TerminalOwner::new(terminal, pty));
            state.terminals.insert(terminal_id.clone(), owner.clone());
            (owner, pending, summary)
        };
        drop(creation);
        if let Err(error) = pending.wait() {
            self.lock().terminals.remove(&terminal_id);
            let _ = self.stop_owner(&owner, true);
            return Err(error);
        }
        self.report_work();
        self.start_pump(&owner);
        Ok(summary)
    }

    pub fn list(&self, session_id: &str) -> Vec<RoomTerminalSummary> {
        let owners = self.owners_for_session(session_id);
        let mut summaries = Vec::new();
        for owner in owners {
            let Ok(mut state) = owner.lock_open() else {
                continue;
            };
            let TerminalState { pty, terminal, .. } = &mut *state;
            let driver = terminal.current_driver(pty);
            summaries.push(terminal.summary(driver));
        }
        summaries.sort_by_key(|summary| summary.created_at_ms);
        summaries
    }

    pub fn snapshot(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<RoomTerminalSnapshot, DaemonError> {
        let owner = self.find(session_id, terminal_id)?;
        let mut state = owner.lock_open()?;
        let TerminalState { pty, terminal, .. } = &mut *state;
        let driver = terminal.current_driver(pty);
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
        let owner = self.find(session_id, terminal_id)?;
        let submits = input.iter().any(|byte| matches!(byte, b'\r' | b'\n'));
        loop {
            // Decide from what the shell has already reported, not from the
            // state of the last tick: a command that just ended still looks
            // like it drives the terminal until its report is read.
            self.pump_terminal(&owner, false);
            {
                // Held until the input is written and the kernel knows about
                // the command it starts.
                let admission = submits.then(|| self.inner.managed.admission());
                let mut state = owner.lock_open()?;
                let TerminalState { pty, terminal, .. } = &mut *state;
                let Some(run_id) = terminal.run_id.clone() else {
                    return Err(ended_error(terminal));
                };
                let now = Instant::now();
                let shell_foreground = terminal.at_prompt(pty, now);
                match terminal.driver.admit(actor, shell_foreground, now) {
                    Admission::Admit => {
                        // Only a line submitted at the prompt starts a command.
                        if let (true, Some(Err(error))) = (shell_foreground, &admission) {
                            return Err(refuse_fenced(error));
                        }
                        // Queue it: waiting for the child to read would hold
                        // this terminal's lock for as long as a program
                        // ignores its input.
                        // A lone interrupt key must still stop the program
                        // when the paste before it is stuck in the queue.
                        let interrupted = match input {
                            [key] => pty.signal_key_behind_blocked_input(&run_id, *key)?,
                            _ => None,
                        };
                        match interrupted {
                            Some(dropped) => terminal.dropped_input_bytes += dropped as u64,
                            None => pty.input_writer(&run_id)?.enqueue_input(input)?,
                        }
                        if *actor == TerminalActor::User {
                            terminal.mark_user_interacted();
                        }
                        terminal
                            .driver
                            .record_write(actor, shell_foreground, input, now);
                        let driver = terminal.driver.observe(shell_foreground);
                        drop(state);
                        self.pump_soon(&owner);
                        self.report_work();
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
        let owner = self.find(session_id, terminal_id)?;
        let mut state = owner.lock_open()?;
        let TerminalState { pty, terminal, .. } = &mut *state;
        let Some(run_id) = terminal.run_id.clone() else {
            return Err(ended_error(terminal));
        };
        pty.resize(&run_id, cols, rows)?;
        terminal.resize(cols, rows);
        let driver = terminal.current_driver(pty);
        let summary = terminal.summary(driver);
        drop(state);
        self.pump_soon(&owner);
        Ok(summary)
    }

    /// Output after `after_offset`, waiting up to `wait` (at most `MAX_WAIT`, so the
    /// answer always arrives inside the gateway's and the CLI's transport deadline).
    pub async fn read_output(
        &self,
        session_id: &str,
        terminal_id: &str,
        after_offset: u64,
        wait: Duration,
    ) -> Result<RoomTerminalOutput, DaemonError> {
        let owner = self.find(session_id, terminal_id)?;
        let deadline = Instant::now() + output_wait(wait);
        loop {
            let changed = owner.output_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = owner.lock_open()?;
                let TerminalState { pty, terminal, .. } = &mut *state;
                let page = terminal.log.read(after_offset, MAX_OUTPUT_PAGE_BYTES);
                let finished = !terminal.is_running() || Instant::now() >= deadline;
                if !page.bytes.is_empty() || page.gap || finished {
                    let driver = terminal.current_driver(pty);
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

    /// Once the user has used a tab, only the user can close it.
    pub fn close(
        &self,
        session_id: &str,
        terminal_id: &str,
        actor: &TerminalActor,
    ) -> Result<(), DaemonError> {
        let owner = self.find(session_id, terminal_id)?;
        {
            let mut state = owner.lock_open()?;
            if *actor != TerminalActor::User && state.terminal.record.user_interacted {
                return Err(refused(format!(
                    "terminal `{terminal_id}` has been used by the user, so only the user can close it"
                )));
            }
            // Reserve removal while ownership and the user-interaction bit
            // are stable; no registry operation waits on a terminal lock.
            state.removed = true;
            self.lock().terminals.remove(terminal_id);
        }
        let _ = self.stop_owner(&owner, true);
        self.settle_outside_jobs(&mut owner.lock().terminal)?;
        self.inner.store.remove(terminal_id)?;
        self.report_work();
        Ok(())
    }

    pub(crate) fn stop_agent(&self, session_id: &str, agent_id: &str) -> Result<(), DaemonError> {
        for owner in self
            .owners_for_session(session_id)
            .into_iter()
            .filter(|owner| owner.opened_by == TerminalActor::Agent(agent_id.to_owned()))
        {
            self.pump_terminal(&owner, true);
            self.stop_owner(&owner, false)?;
            let pending = {
                let mut state = owner.lock();
                if state.removed {
                    continue;
                }
                let pending = self
                    .inner
                    .store
                    .enqueue_save(&state.terminal.persisted_record())?;
                state.terminal.unsaved_since = None;
                pending
            };
            pending.wait()?;
        }
        self.report_work();
        Ok(())
    }

    /// The Room ended: detach its registry entries before stopping their work.
    pub fn end_session(&self, session_id: &str) {
        let removed = {
            let _creation = self
                .inner
                .lifecycle
                .lock()
                .expect("terminal lifecycle poisoned");
            let mut state = self.lock();
            let ids: Vec<_> = state
                .terminals
                .iter()
                .filter(|(_, owner)| owner.session_id == session_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .map(|id| {
                    let owner = state
                        .terminals
                        .remove(&id)
                        .expect("selected terminal exists");
                    (id, owner)
                })
                .collect::<Vec<_>>()
        };
        for (id, owner) in removed {
            let _ = self.stop_owner(&owner, true);
            if let Err(error) = self.settle_outside_jobs(&mut owner.lock().terminal) {
                crate::logging::error_with_fields(
                    "room_terminal.events",
                    "ended terminal completion could not be committed",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
            if let Err(error) = self.inner.store.remove(&id) {
                crate::logging::error_with_fields(
                    "room_terminal.remove",
                    "ended terminal record could not be removed",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
        }
        self.report_work();
    }

    /// Ask to be told when the terminal finishes a program, prints a text or
    /// waits for input. `after_offset` is the last output offset the owner
    /// read: a condition that already held after it fires at once. The event
    /// goes to the `CompletionEvents` sink.
    pub fn watch(
        &self,
        session_id: &str,
        terminal_id: &str,
        watch_owner: WatchOwner,
        watches: Vec<RoomTerminalWatch>,
        after_offset: Option<u64>,
    ) -> Result<(), DaemonError> {
        if watches.is_empty() {
            return Err(refused(
                "a notification needs at least one condition".to_string(),
            ));
        }
        let immediate = {
            let owner = self.find(session_id, terminal_id)?;
            let mut state = owner.lock_open()?;
            let TerminalState { pty, terminal, .. } = &mut *state;
            if !terminal.is_running() {
                return Err(ended_error(terminal));
            }
            let now = Instant::now();
            let program_running = !terminal.at_prompt(pty, now);
            let mut immediate = None;
            for watch in watches {
                if let Some(kind) = terminal.arm_watch(
                    watch_owner.clone(),
                    watch,
                    after_offset,
                    now,
                    program_running,
                ) {
                    terminal.watches.disarm(&watch_owner.wake_id);
                    immediate = Some(event(
                        terminal,
                        watch_owner.clone(),
                        kind,
                        terminal.screen.visible_text(),
                    ));
                    break;
                }
            }
            immediate
        };
        if let Some(event) = immediate {
            self.inner.events.deliver(event);
        }
        Ok(())
    }

    /// The wake was cancelled: forget its watches.
    pub fn cancel_watch(&self, session_id: &str, terminal_id: &str, wake_id: &str) {
        if let Ok(owner) = self.find(session_id, terminal_id) {
            owner.lock().terminal.watches.disarm(wake_id);
        }
    }

    /// Block until one of `watches` holds, up to `timeout` (never more than
    /// `MAX_WAIT`). On timeout the program is still running and nothing is
    /// lost: arm a notification next. No lock is held while waiting.
    pub async fn wait(
        &self,
        session_id: &str,
        terminal_id: &str,
        agent_id: &str,
        watches: Vec<RoomTerminalWatch>,
        after_offset: Option<u64>,
        timeout: Duration,
    ) -> Result<RoomTerminalWaitResult, DaemonError> {
        if watches.is_empty() {
            return Err(refused("wait needs at least one condition".to_string()));
        }
        let owner = self.find(session_id, terminal_id)?;
        let (wake_id, receiver) = {
            let mut state = owner.lock_open()?;
            let TerminalState {
                pty,
                terminal,
                waiters,
                next_wait,
                ..
            } = &mut *state;
            if !terminal.is_running() {
                return Ok(ended_wait_result(terminal, pty));
            }
            *next_wait += 1;
            let owner = WatchOwner {
                wake_id: format!("wait:{terminal_id}:{next_wait}"),
                agent_id: agent_id.to_string(),
            };
            let now = Instant::now();
            let program_running = !terminal.at_prompt(pty, now);
            for watch in watches {
                if let Some(kind) =
                    terminal.arm_watch(owner.clone(), watch, after_offset, now, program_running)
                {
                    terminal.watches.disarm(&owner.wake_id);
                    return Ok(wait_result(terminal, pty, Some(kind)));
                }
            }
            let (sender, receiver) = oneshot::channel();
            waiters.insert(owner.wake_id.clone(), sender);
            (owner.wake_id, receiver)
        };
        let fired = tokio::time::timeout(timeout.min(MAX_WAIT), receiver).await;
        let mut state = owner.lock_open()?;
        let TerminalState {
            pty,
            terminal,
            waiters,
            ..
        } = &mut *state;
        waiters.remove(&wake_id);
        if let Ok(Err(_)) = fired {
            return Err(refused(format!(
                "terminal `{terminal_id}` was closed while waiting"
            )));
        }
        match fired {
            Ok(Ok(event)) => Ok(wait_result(terminal, pty, Some(event.kind))),
            Ok(Err(_)) => unreachable!("closed waits return above"),
            Err(_) => {
                terminal.watches.disarm(&wake_id);
                Ok(wait_result(terminal, pty, None))
            }
        }
    }

    /// Let a managed kernel see terminal commands as work: they hold its stop
    /// back and are refused once a stop is reserved.
    pub(crate) fn link_managed_kernel(
        &self,
        gate: Arc<ManagedKernelQuiescenceGate>,
        work_changed: std::sync::Weak<dyn Fn() + Send + Sync>,
    ) {
        self.inner.managed.link(gate, work_changed);
    }

    /// Whether any terminal runs work: a program holds its foreground, the
    /// shell reports a command in flight, a line was just submitted, or a
    /// background job still runs in the shell's session.
    pub fn has_running_work(&self) -> bool {
        let owners = self.owners();
        owners.iter().any(|owner| owner.has_running_work())
    }

    /// Report the answer now instead of at the next pump tick: a background
    /// job ends without any kernel event, so a reader of the managed activity
    /// would otherwise see a change nothing recorded yet.
    pub(crate) fn report_work(&self) {
        self.inner.managed.report(|| self.has_running_work());
    }

    fn find(&self, session_id: &str, terminal_id: &str) -> Result<Arc<TerminalOwner>, DaemonError> {
        self.lock()
            .terminals
            .get(terminal_id)
            .filter(|owner| owner.session_id == session_id)
            .cloned()
            .ok_or_else(|| refused(format!("this Room has no terminal `{terminal_id}`")))
    }

    fn owners(&self) -> Vec<Arc<TerminalOwner>> {
        self.lock().terminals.values().cloned().collect()
    }

    fn owners_for_session(&self, session_id: &str) -> Vec<Arc<TerminalOwner>> {
        self.lock()
            .terminals
            .values()
            .filter(|owner| owner.session_id == session_id)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(super) fn terminal_for_test(&self, id: &str) -> Arc<TerminalOwner> {
        self.lock().terminals[id].clone()
    }

    #[cfg(test)]
    pub(super) fn pause_writer_for_test(
        &self,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) -> super::store::PendingWrite {
        self.inner.store.pause_writer(entered, release)
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .expect("room terminal state poisoned")
    }
}

fn event(
    terminal: &Terminal,
    owner: WatchOwner,
    kind: RoomTerminalEventKind,
    screen_text: String,
) -> RoomTerminalEvent {
    RoomTerminalEvent {
        terminal_id: terminal.record.terminal_id.clone(),
        session_id: terminal.record.session_id.clone(),
        wake_id: owner.wake_id,
        armed_by: owner.agent_id,
        kind,
        screen_text,
    }
}

fn wait_result(
    terminal: &mut Terminal,
    pty: &PtyManager,
    fired: Option<RoomTerminalEventKind>,
) -> RoomTerminalWaitResult {
    let driver = terminal.current_driver(pty);
    RoomTerminalWaitResult {
        fired,
        screen_text: terminal.screen.visible_text(),
        output_offset: terminal.log.end_offset(),
        terminal: terminal.summary(driver),
    }
}

/// A terminal that already ended answers a wait with how it ended.
fn ended_wait_result(terminal: &mut Terminal, pty: &PtyManager) -> RoomTerminalWaitResult {
    let (exit_code, signal) = match &terminal.record.ended {
        Some(RoomTerminalOutcome::ProgramExited { exit_code, signal }) => {
            (*exit_code, signal.clone())
        }
        _ => (None, None),
    };
    wait_result(
        terminal,
        pty,
        Some(match &terminal.record.ended {
            Some(RoomTerminalOutcome::MemoryLimit { limit_bytes }) => {
                RoomTerminalEventKind::MemoryLimit {
                    limit_bytes: *limit_bytes,
                }
            }
            Some(RoomTerminalOutcome::OutsideEnd) => RoomTerminalEventKind::OutsideEnd,
            _ => RoomTerminalEventKind::Finished { exit_code, signal },
        }),
    )
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

fn next_title(in_room: &[&Arc<TerminalOwner>]) -> String {
    (1..)
        .map(|number| format!("Terminal {number}"))
        .find(|title| in_room.iter().all(|terminal| terminal.title != *title))
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

/// The terminal service's own sentence about what it will not do. It names only the
/// terminal id the caller sent, so a grant holder reads it as written (`is_public_code`).
pub(super) fn refused(message: String) -> DaemonError {
    DaemonError::MachineRequest {
        code: REFUSED_CODE.into(),
        message,
    }
}

/// The managed kernel is about to stop: say why a command cannot start.
fn refuse_fenced(error: &DaemonError) -> DaemonError {
    refused(format!(
        "this kernel is stopping, so it starts no commands: {error}"
    ))
}
