//! One terminal's state: its PTY handle, screen, output and driver.

use std::time::{Duration, Instant};

use base64::Engine as _;

use super::driver::DriverTracker;
use super::events::{
    FinishBinding, Fired, OutputMatcher, WatchOwner, WatchSet, QUIET_BEFORE_WAITING,
};
use super::output_log::OutputLog;
use super::plain_text::PlainText;
use super::screen::Screen;
use super::session_sweep::SessionSweep;
use super::shell_marks::{MarkScanner, ShellMark};
use super::store::TerminalRecord;
use super::types::{
    RoomTerminalDriver, RoomTerminalOutcome, RoomTerminalSnapshot, RoomTerminalStatus,
    RoomTerminalSummary,
};
use super::types::{RoomTerminalEventKind, RoomTerminalWatch};
use crate::pty::PtyManager;

/// Longest a shell may take to reach its first prompt and still count as starting.
const FIRST_PROMPT_LIMIT: Duration = Duration::from_secs(30);

pub(super) struct Terminal {
    pub record: TerminalRecord,
    /// The PTY key while the shell runs.
    pub run_id: Option<String>,
    pub shell_pid: Option<i32>,
    pub session_sweep: Option<SessionSweep>,
    pub screen: Screen,
    pub log: OutputLog,
    pub plain: PlainText,
    pub driver: DriverTracker,
    pub watches: WatchSet,
    pub last_output_at: Instant,
    pub program_running: bool,
    pub marks: MarkScanner,
    /// The shell reports finish marks (including its first prompt): sampling
    /// must not race commands that keep the shell in the foreground.
    pub mark_tracking: bool,
    /// Until this instant a shell that reports marks is still running its login
    /// files, which are not anyone's command; its first prompt mark clears it.
    pub first_prompt_deadline: Option<Instant>,
    /// A command line the shell reported started and not yet finished.
    pub command_in_flight: bool,
    pub waiting_reported: bool,
    /// Input an interrupt discarded before the terminal's program read it.
    pub dropped_input_bytes: u64,
    pub unsaved_since: Option<Instant>,
    /// The last time a program returned to the prompt (or the shell exited).
    pub last_program_end: Option<ProgramEnd>,
    pub outside_ends: Vec<ProgramEnd>,
    /// Commands that began, in order: the n-th command answers the n-th line.
    pub commands_started: u64,
}

#[derive(Clone)]
pub(super) struct ProgramEnd {
    /// Output offset when it ended: a watch armed after reading less than
    /// this still sees the end.
    pub output_offset: u64,
    /// The number of the command that ended. A wait for a later command must
    /// not see this end. `None` for a shell exit, which ends whatever was
    /// running.
    pub command: Option<u64>,
    pub at: Instant,
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
}

impl Terminal {
    pub(super) fn running(
        record: TerminalRecord,
        run_id: String,
        shell_pid: Option<i32>,
        reports_first_prompt: bool,
    ) -> Self {
        let screen = Screen::new(record.rows, record.cols);
        let mut terminal = Self::new(record, Some(run_id), shell_pid, screen);
        terminal.first_prompt_deadline =
            reports_first_prompt.then(|| Instant::now() + FIRST_PROMPT_LIMIT);
        if terminal.runs_a_command() {
            terminal.driver = DriverTracker::owned_by(terminal.record.opened_by.clone());
        }
        terminal
    }

    /// An ended terminal whose scrollback survived a kernel restart.
    pub(super) fn recovered(record: TerminalRecord) -> Self {
        let screen = Screen::replay(
            record.rows,
            record.cols,
            &record.scrollback_text,
            &record.screen_text,
        );
        Self::new(record, None, None, screen)
    }

    fn new(
        record: TerminalRecord,
        run_id: Option<String>,
        shell_pid: Option<i32>,
        screen: Screen,
    ) -> Self {
        Self {
            record,
            run_id,
            shell_pid,
            session_sweep: shell_pid.map(SessionSweep::capture),
            screen,
            log: OutputLog::default(),
            plain: PlainText::new(),
            driver: DriverTracker::default(),
            watches: WatchSet::default(),
            last_output_at: Instant::now(),
            program_running: false,
            marks: MarkScanner::default(),
            mark_tracking: false,
            first_prompt_deadline: None,
            command_in_flight: false,
            waiting_reported: false,
            dropped_input_bytes: 0,
            unsaved_since: None,
            last_program_end: None,
            outside_ends: Vec::new(),
            commands_started: 0,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.run_id.is_some()
    }

    pub(super) fn apply_output(&mut self, bytes: &[u8], now: Instant) -> Fired {
        self.screen.feed(bytes);
        self.log.append(bytes);

        self.last_output_at = now;
        self.waiting_reported = false;
        self.unsaved_since.get_or_insert(now);
        let mut fired = Fired::new();
        // Check every bounded piece before the text window trims it. The
        // watch keeps its own overlap if its needle is longer than the window.
        for part in bytes.chunks(4096) {
            self.plain.append(part);
            fired.extend(self.watches.on_output(&self.plain));
        }
        for mark in self.marks.feed(bytes) {
            fired.extend(self.on_mark(mark));
        }
        fired
    }

    pub(super) fn record_program_end(&mut self, end: ProgramEnd) {
        self.outside_ends.push(end.clone());
        self.last_program_end = Some(end);
    }

    fn on_mark(&mut self, mark: ShellMark) -> Fired {
        match mark {
            ShellMark::CommandStarted => {
                // Login files never report a start: this is a real command,
                // even when the shell has not reported a prompt yet (zsh
                // installs its finish hook during the first prompt).
                self.first_prompt_deadline = None;
                self.command_in_flight = true;
                self.commands_started += 1;
                Fired::new()
            }
            ShellMark::CommandFinished { exit_code } => {
                self.mark_tracking = true;
                self.first_prompt_deadline = None;
                // The first prompt reports before any command has run.
                if !self.command_in_flight && !self.program_running {
                    return Fired::new();
                }
                self.command_in_flight = false;
                let command = Some(self.commands_started);
                self.record_program_end(ProgramEnd {
                    output_offset: self.log.end_offset(),
                    command,
                    at: Instant::now(),
                    exit_code,
                    signal: None,
                });
                self.watches.on_command_finished(exit_code, command)
            }
        }
    }

    /// A command or program holds the terminal: the shell said so, or a
    /// program other than the shell owns the foreground.
    pub(super) fn is_busy(&self, pty: &PtyManager) -> bool {
        self.command_in_flight || !self.shell_is_foreground(pty)
    }

    /// The user used this tab: from now on only the user can close it, and
    /// the next save keeps that across a restart.
    pub(super) fn mark_user_interacted(&mut self) {
        if !self.record.user_interacted {
            self.record.user_interacted = true;
            self.unsaved_since.get_or_insert_with(Instant::now);
        }
    }

    pub(super) fn end(&mut self, outcome: RoomTerminalOutcome) {
        self.run_id = None;
        self.record.ended = Some(outcome);
        self.unsaved_since = Some(Instant::now());
    }

    /// Whether the shell itself, not a program it started, owns the terminal.
    pub(super) fn shell_is_foreground(&self, pty: &PtyManager) -> bool {
        // A terminal opened with a command has no prompt: its program is the
        // foreground until it exits.
        if self.runs_a_command() {
            return false;
        }
        match (&self.run_id, self.shell_pid) {
            (Some(run_id), Some(shell_pid)) => {
                pty.foreground_process_group(run_id).ok().flatten() == Some(shell_pid)
            }
            _ => true,
        }
    }

    /// The shell is still running its login files. Input typed now waits in the
    /// terminal for the first prompt, and nobody drives the files themselves.
    pub(super) fn starting(&self, now: Instant) -> bool {
        self.first_prompt_deadline
            .is_some_and(|deadline| now < deadline)
    }

    /// The shell takes a line now or soon: it is at its prompt, or still on
    /// its way there. Programs the login files run do not hold the terminal.
    pub(super) fn at_prompt(&self, pty: &PtyManager, now: Instant) -> bool {
        !self.is_busy(pty) || self.starting(now)
    }

    pub(super) fn current_driver(&mut self, pty: &PtyManager) -> RoomTerminalDriver {
        let at_prompt = self.at_prompt(pty, Instant::now());
        self.driver.observe(at_prompt)
    }

    /// Arm a watch, unless its condition already holds for what the caller
    /// has not seen yet (`after_offset` is the last output offset they read):
    /// then the event is returned at once and nothing is armed.
    pub(super) fn arm_watch(
        &mut self,
        owner: WatchOwner,
        watch: RoomTerminalWatch,
        after_offset: Option<u64>,
        now: Instant,
        program_running: bool,
    ) -> Option<RoomTerminalEventKind> {
        let binding = FinishBinding {
            line: self.driver.latest_line(&owner.agent_id),
            after_offset,
        };
        let output = match &watch {
            RoomTerminalWatch::OutputContains { text } => {
                match self.replay_output(text, after_offset) {
                    Ok(unmatched) => OutputMatcher::after(&self.plain, unmatched),
                    Err(kind) => return Some(kind),
                }
            }
            _ => {
                if let Some(kind) = self.already_true(&watch, binding, now, program_running) {
                    return Some(kind);
                }
                OutputMatcher::new(&self.plain)
            }
        };
        self.watches
            .arm(owner, watch, output, program_running, binding);
        None
    }

    /// Whether `text` already appeared in the output after `after_offset`.
    /// If not, the unmatched end is returned: it may be the start of a match
    /// that the live output completes.
    fn replay_output(
        &self,
        text: &str,
        after_offset: Option<u64>,
    ) -> Result<String, RoomTerminalEventKind> {
        let Some(after_offset) = after_offset else {
            return Ok(String::new());
        };
        let page = self.log.read(after_offset, usize::MAX);
        if page.gap {
            return Ok(String::new());
        }
        let mut plain = PlainText::new();
        let mut output = OutputMatcher::new(&plain);
        for part in page.bytes.chunks(4096) {
            plain.append(part);
            if output.matches(&plain, text) {
                return Err(RoomTerminalEventKind::OutputMatched {
                    text: text.to_string(),
                });
            }
        }
        Ok(output.into_tail())
    }

    fn already_true(
        &self,
        watch: &RoomTerminalWatch,
        binding: FinishBinding,
        now: Instant,
        program_running: bool,
    ) -> Option<RoomTerminalEventKind> {
        match watch {
            RoomTerminalWatch::Finished => {
                let end = self.last_program_end.as_ref()?;
                let counts = match (end.command, binding.line) {
                    (Some(command), Some(line)) => command >= line,
                    _ => true,
                };
                (end.output_offset > binding.after_offset? && counts).then(|| {
                    RoomTerminalEventKind::Finished {
                        exit_code: end.exit_code,
                        signal: end.signal.clone(),
                    }
                })
            }
            RoomTerminalWatch::OutputContains { .. } => None,
            RoomTerminalWatch::WaitingForInput => (program_running
                && now.duration_since(self.last_output_at) >= QUIET_BEFORE_WAITING)
                .then_some(RoomTerminalEventKind::WaitingForInput),
        }
    }

    pub(super) fn resize(&mut self, cols: u16, rows: u16) {
        self.record.cols = cols;
        self.record.rows = rows;
        self.screen.resize(rows, cols);
    }

    pub(super) fn summary(&self, driver: RoomTerminalDriver) -> RoomTerminalSummary {
        let record = &self.record;
        RoomTerminalSummary {
            terminal_id: record.terminal_id.clone(),
            session_id: record.session_id.clone(),
            title: record.title.clone(),
            program: record.program.clone(),
            working_directory: record.working_directory.clone(),
            cols: record.cols,
            rows: record.rows,
            status: match &record.ended {
                None => RoomTerminalStatus::Running,
                Some(outcome) => RoomTerminalStatus::Ended {
                    outcome: outcome.clone(),
                    description: outcome.description(),
                },
            },
            driver,
            opened_by: record.opened_by.clone(),
            user_interacted: record.user_interacted,
            created_at_ms: record.created_at_ms,
            output_offset: self.log.end_offset(),
            dropped_input_bytes: self.dropped_input_bytes,
            host_agent: None,
        }
    }

    pub(super) fn snapshot(&mut self, driver: RoomTerminalDriver) -> RoomTerminalSnapshot {
        let text = self.screen.text();
        RoomTerminalSnapshot {
            terminal: self.summary(driver),
            cursor_row: text.cursor_row,
            cursor_col: text.cursor_col,
            screen_text: text.screen,
            scrollback_text: text.scrollback,
            screen_ansi_base64: base64::engine::general_purpose::STANDARD.encode(text.ansi),
            output_offset: self.log.end_offset(),
        }
    }

    /// The record to persist now: metadata plus the text of the screen.
    pub(super) fn persisted_record(&mut self) -> TerminalRecord {
        let text = self.screen.text();
        TerminalRecord {
            scrollback_text: text.scrollback,
            screen_text: text.screen,
            ..self.record.clone()
        }
    }

    /// A terminal opened with a command belongs to its opener until it exits.
    pub(super) fn runs_a_command(&self) -> bool {
        self.record.command.is_some()
    }
}
