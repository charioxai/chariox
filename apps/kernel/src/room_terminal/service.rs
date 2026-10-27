//! Room-scoped terminals on the kernel's `PtyManager`.
//!
//! The service is authority-free: callers have already decided who may
//! reach a Room's terminals and which actor a request speaks for.

use super::REFUSED_CODE;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore;
use tokio::sync::{oneshot, Notify};

use super::driver::Admission;
use super::events::{BoundEvents, CompletionEvents, WatchOwner};
use super::managed_work::ManagedWork;
use super::session_sweep::SessionSweep;
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
    store: TerminalStore,
    events: BoundEvents,
    outside_events: std::sync::RwLock<Option<crate::durable_state::DurableKernelStateStore>>,
    shell_integration: ShellIntegration,
    managed: ManagedWork,
    output_changed: Notify,
    pump_started: AtomicBool,
    /// Wakes the pump now; input and a resize have an answer coming.
    pump_wake: Notify,
    /// The pump runs at `ACTIVE_PUMP_INTERVAL` until then.
    pump_active_until: Mutex<Instant>,
}

pub(super) struct State {
    pty: PtyManager,
    pub(super) terminals: BTreeMap<String, Terminal>,
    /// Blocking waits by wake id with their terminal; their events do not go
    /// to `events`.
    waiters: BTreeMap<String, (String, oneshot::Sender<RoomTerminalEvent>)>,
    next_wait: u64,
}

impl RoomTerminalService {
    /// Terminals that ran when the kernel last stopped come back as ended by
    /// the restart, scrollback kept.
    pub fn new(store_root: PathBuf) -> Self {
        let shell_integration = ShellIntegration::install(&store_root.with_extension("shell"));
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
                    waiters: BTreeMap::new(),
                    next_wait: 0,
                }),
                store,
                events: BoundEvents::default(),
                outside_events: std::sync::RwLock::new(None),
                shell_integration,
                managed: ManagedWork::default(),
                output_changed: Notify::new(),
                pump_started: AtomicBool::new(false),
                pump_wake: Notify::new(),
                pump_active_until: Mutex::new(Instant::now()),
            }),
        }
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
        let user_interacted = spec.opened_by == TerminalActor::User;
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
        let mut env = BTreeMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        let mut reports_first_prompt = false;
        if spec.command.is_none() {
            let marks = self.inner.shell_integration.environment(&program);
            reports_first_prompt = !marks.is_empty();
            env.extend(marks);
        }
        state.pty.spawn_interactive(PtySpawnRequest {
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
        if let Err(error) = self.inner.store.save(&record) {
            let _ = state.pty.remove_process(&run_id);
            return Err(error);
        }
        let terminal = Terminal::running(record, run_id, shell_pid, reports_first_prompt);
        let summary = terminal.summary(RoomTerminalDriver::Free);
        state.terminals.insert(terminal_id, terminal);
        drop(state);
        self.report_work();
        self.start_pump();
        Ok(summary)
    }

    pub fn list(&self, session_id: &str) -> Vec<RoomTerminalSummary> {
        let mut state = self.lock();
        let State { pty, terminals, .. } = &mut *state;
        let mut summaries: Vec<_> = terminals
            .values_mut()
            .filter(|terminal| terminal.record.session_id == session_id)
            .map(|terminal| {
                let driver = terminal.current_driver(pty);
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
        let State { pty, terminals, .. } = &mut *state;
        let terminal = find(terminals, session_id, terminal_id)?;
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
        let submits = input.iter().any(|byte| matches!(byte, b'\r' | b'\n'));
        loop {
            {
                // Held until the input is written and the kernel knows about
                // the command it starts.
                let admission = submits.then(|| self.inner.managed.admission());
                let mut state = self.lock();
                let State { pty, terminals, .. } = &mut *state;
                let terminal = find(terminals, session_id, terminal_id)?;
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
                        // this lock, and with it every Room terminal, for as
                        // long as a program ignores its input.
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
                        self.pump_soon();
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
        actor: &TerminalActor,
        cols: u16,
        rows: u16,
    ) -> Result<RoomTerminalSummary, DaemonError> {
        validate_size(cols, rows)?;
        let mut state = self.lock();
        let State { pty, terminals, .. } = &mut *state;
        let terminal = find(terminals, session_id, terminal_id)?;
        let Some(run_id) = terminal.run_id.clone() else {
            return Err(ended_error(terminal));
        };
        pty.resize(&run_id, cols, rows)?;
        terminal.resize(cols, rows);
        if *actor == TerminalActor::User {
            terminal.mark_user_interacted();
        }
        let driver = terminal.current_driver(pty);
        self.pump_soon();
        Ok(terminal.summary(driver))
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
        let deadline = Instant::now() + output_wait(wait);
        loop {
            let changed = self.inner.output_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.lock();
                let State { pty, terminals, .. } = &mut *state;
                let terminal = find(terminals, session_id, terminal_id)?;
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
        let mut state = self.lock();
        let terminal = find(&mut state.terminals, session_id, terminal_id)?;
        if *actor != TerminalActor::User && terminal.record.user_interacted {
            return Err(refused(format!(
                "terminal `{terminal_id}` has been used by the user, so only the user can close it"
            )));
        }
        self.remove(&mut state, terminal_id);
        drop(state);
        self.report_work();
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
        drop(state);
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
        owner: WatchOwner,
        watches: Vec<RoomTerminalWatch>,
        after_offset: Option<u64>,
    ) -> Result<(), DaemonError> {
        if watches.is_empty() {
            return Err(refused(
                "a notification needs at least one condition".to_string(),
            ));
        }
        let immediate = {
            let mut state = self.lock();
            let State { pty, terminals, .. } = &mut *state;
            let terminal = find(terminals, session_id, terminal_id)?;
            if !terminal.is_running() {
                return Err(ended_error(terminal));
            }
            let now = Instant::now();
            let program_running = !terminal.at_prompt(pty, now);
            let mut immediate = None;
            for watch in watches {
                if let Some(kind) =
                    terminal.arm_watch(owner.clone(), watch, after_offset, now, program_running)
                {
                    terminal.watches.disarm(&owner.wake_id);
                    immediate = Some(event(
                        terminal,
                        owner.clone(),
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
        let mut state = self.lock();
        if let Ok(terminal) = find(&mut state.terminals, session_id, terminal_id) {
            terminal.watches.disarm(wake_id);
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
        let (wake_id, receiver) = {
            let mut state = self.lock();
            let State {
                pty,
                terminals,
                waiters,
                next_wait,
            } = &mut *state;
            let terminal = find(terminals, session_id, terminal_id)?;
            if !terminal.is_running() {
                return Ok(ended_wait_result(terminal, pty));
            }
            *next_wait += 1;
            let owner = WatchOwner {
                wake_id: format!("wait:{next_wait}"),
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
            waiters.insert(owner.wake_id.clone(), (terminal_id.to_string(), sender));
            (owner.wake_id, receiver)
        };
        let fired = tokio::time::timeout(timeout.min(MAX_WAIT), receiver).await;
        let mut state = self.lock();
        let State {
            pty,
            terminals,
            waiters,
            ..
        } = &mut *state;
        waiters.remove(&wake_id);
        if let Ok(Err(_)) = fired {
            return Err(refused(format!(
                "terminal `{terminal_id}` was closed while waiting"
            )));
        }
        let terminal = find(terminals, session_id, terminal_id)?;
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
        let sweeps = {
            let mut state = self.lock();
            let State { pty, terminals, .. } = &mut *state;
            let now = Instant::now();
            let mut sweeps = Vec::new();
            for terminal in terminals.values().filter(|terminal| terminal.is_running()) {
                if terminal.is_busy(pty) || terminal.driver.launching(now) {
                    return true;
                }
                sweeps.extend(terminal.session_sweep);
            }
            sweeps
        };
        // Looking through the process table needs no terminal lock.
        sweeps.iter().any(SessionSweep::has_jobs)
    }

    /// Report the answer now instead of at the next pump tick: a background
    /// job ends without any kernel event, so a reader of the managed activity
    /// would otherwise see a change nothing recorded yet.
    pub(crate) fn report_work(&self) {
        self.inner.managed.report(|| self.has_running_work());
    }

    fn remove(&self, state: &mut State, terminal_id: &str) {
        state
            .waiters
            .retain(|_, (waiting_on, _)| waiting_on != terminal_id);
        if let Some((run_id, sweep)) = state.terminals.remove(terminal_id).and_then(|terminal| {
            terminal
                .run_id
                .map(|run_id| (run_id, terminal.session_sweep))
        }) {
            if let Some(sweep) = sweep {
                sweep.stop();
            }
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
            let mut last_full = Instant::now();
            loop {
                let full = last_full.elapsed() >= PUMP_INTERVAL;
                if full {
                    last_full = Instant::now();
                }
                service.pump_pass(full);
                let interval = if service.pump_is_active() {
                    ACTIVE_PUMP_INTERVAL
                } else {
                    PUMP_INTERVAL
                };
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    () = service.inner.pump_wake.notified() => {}
                }
            }
        });
    }

    /// Something was just typed or resized: pump now and keep pumping fast
    /// while the answer arrives.
    fn pump_soon(&self) {
        self.extend_active_pump();
        self.inner.pump_wake.notify_one();
    }

    fn extend_active_pump(&self) {
        *self
            .inner
            .pump_active_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now() + ACTIVE_PUMP_WINDOW;
    }

    fn pump_is_active(&self) -> bool {
        Instant::now()
            < *self
                .inner
                .pump_active_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
    }

    /// Move PTY output into screens, end terminals whose shell exited, track
    /// the driver, fire watches and persist what changed.
    pub(super) fn pump_once(&self) {
        self.pump_pass(true);
    }

    /// `full` also looks at the foreground program and the process table;
    /// the quick passes between full ones only move output.
    fn pump_pass(&self, full: bool) {
        let mut output_changed = false;
        let mut notifications: Vec<RoomTerminalEvent> = Vec::new();
        let mut wakes: Vec<(oneshot::Sender<RoomTerminalEvent>, RoomTerminalEvent)> = Vec::new();
        {
            let mut state = self.lock();
            let State {
                pty,
                terminals,
                waiters,
                ..
            } = &mut *state;
            let now = Instant::now();
            for terminal in terminals.values_mut() {
                let pumped = if full {
                    terminal.pump(pty, now)
                } else {
                    terminal.pump_output(pty, now)
                };
                output_changed |= pumped.output_changed;
                if !pumped.fired.is_empty() {
                    let screen_text = terminal.screen.visible_text();
                    for (owner, kind) in pumped.fired {
                        let fired = event(terminal, owner.clone(), kind, screen_text.clone());
                        match waiters.remove(&owner.wake_id) {
                            Some((_, waiter)) => wakes.push((waiter, fired)),
                            None => notifications.push(fired),
                        }
                    }
                }
                self.publish_outside_jobs(terminal);
                self.persist_if_due(terminal, now);
            }
        }
        if full {
            self.report_work();
        }
        if output_changed {
            self.extend_active_pump();
            self.inner.output_changed.notify_waiters();
        }
        for (waiter, fired) in wakes {
            let _ = waiter.send(fired);
        }
        for fired in notifications {
            self.inner.events.deliver(fired);
        }
    }

    /// Stop the terminal sessions, keeping their screens and records so the
    /// next start reports them as ended by the restart.
    pub fn shutdown(&self) {
        self.pump_once();
        let mut state = self.lock();
        let State {
            pty,
            terminals,
            waiters,
            ..
        } = &mut *state;
        for terminal in terminals.values_mut() {
            if let Some(run_id) = &terminal.run_id {
                if let Some(sweep) = &terminal.session_sweep {
                    sweep.stop();
                }
                if let Err(error) = pty.remove_process(run_id) {
                    crate::logging::error_with_fields(
                        "room_terminal.shutdown",
                        "terminal process could not be removed",
                        serde_json::json!({ "terminal_id": terminal.record.terminal_id, "error": error.to_string() }),
                    );
                }
                terminal.end(RoomTerminalOutcome::KernelRestarted);
            }
            self.persist(terminal);
        }
        waiters.clear();
        drop(state);
        self.report_work();
    }

    fn persist_if_due(&self, terminal: &mut Terminal, now: Instant) {
        let due = terminal.unsaved_since.is_some_and(|since| {
            !terminal.is_running() || now.duration_since(since) >= PERSIST_INTERVAL
        });
        if due {
            self.persist(terminal);
        }
    }

    fn persist(&self, terminal: &mut Terminal) {
        terminal.unsaved_since = None;
        if let Err(error) = self.inner.store.save(&terminal.persisted_record()) {
            crate::logging::error_with_fields(
                "room_terminal.persist",
                "terminal scrollback could not be saved",
                serde_json::json!({ "error": error.to_string() }),
            );
        }
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
        Some(RoomTerminalEventKind::Finished { exit_code, signal }),
    )
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

fn validate_size(cols: u16, rows: u16) -> Result<(), DaemonError> {
    if (1..=500).contains(&cols) && (1..=200).contains(&rows) {
        Ok(())
    } else {
        Err(refused(
            "terminal size must be 1-500 columns by 1-200 rows".to_string(),
        ))
    }
}

fn next_title(in_room: &[&Terminal]) -> String {
    (1..)
        .map(|number| format!("Terminal {number}"))
        .find(|title| {
            in_room
                .iter()
                .all(|terminal| terminal.record.title != *title)
        })
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
