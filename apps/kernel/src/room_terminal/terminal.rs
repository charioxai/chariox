//! One terminal's state: its PTY handle, screen, output and driver.

use std::time::Instant;

use base64::Engine as _;

use super::driver::DriverTracker;
use super::events::{WatchOwner, WatchSet, QUIET_BEFORE_WAITING};
use super::output_log::OutputLog;
use super::plain_text::PlainText;
use super::screen::Screen;
use super::store::TerminalRecord;
use super::types::{
    RoomTerminalDriver, RoomTerminalOutcome, RoomTerminalSnapshot, RoomTerminalStatus,
    RoomTerminalSummary,
};
use super::types::{RoomTerminalEventKind, RoomTerminalWatch};
use crate::pty::PtyManager;

pub(super) struct Terminal {
    pub record: TerminalRecord,
    /// The PTY key while the shell runs.
    pub run_id: Option<String>,
    pub shell_pid: Option<i32>,
    pub screen: Screen,
    pub log: OutputLog,
    pub plain: PlainText,
    pub driver: DriverTracker,
    pub watches: WatchSet,
    pub last_output_at: Instant,
    pub program_running: bool,
    pub waiting_reported: bool,
    pub unsaved_since: Option<Instant>,
    /// The last time a program returned to the prompt (or the shell exited).
    pub last_program_end: Option<ProgramEnd>,
}

#[derive(Clone)]
pub(super) struct ProgramEnd {
    /// Output offset when it ended: a watch armed after reading less than
    /// this still sees the end.
    pub output_offset: u64,
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
}

impl Terminal {
    pub(super) fn running(record: TerminalRecord, run_id: String, shell_pid: Option<i32>) -> Self {
        let screen = Screen::new(record.rows, record.cols);
        let mut terminal = Self::new(record, Some(run_id), shell_pid, screen);
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
            screen,
            log: OutputLog::default(),
            plain: PlainText::new(),
            driver: DriverTracker::default(),
            watches: WatchSet::default(),
            last_output_at: Instant::now(),
            program_running: false,
            waiting_reported: false,
            unsaved_since: None,
            last_program_end: None,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.run_id.is_some()
    }

    pub(super) fn apply_output(&mut self, bytes: &[u8], now: Instant) {
        self.screen.feed(bytes);
        self.log.append(bytes);
        self.plain.append(bytes);
        self.last_output_at = now;
        self.waiting_reported = false;
        self.unsaved_since.get_or_insert(now);
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

    pub(super) fn current_driver(&mut self, pty: &PtyManager) -> RoomTerminalDriver {
        let shell_foreground = self.shell_is_foreground(pty);
        self.driver.observe(shell_foreground)
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
        if let Some(kind) = self.already_true(&watch, after_offset, now, program_running) {
            return Some(kind);
        }
        self.watches.arm(owner, watch, &self.plain, program_running);
        None
    }

    fn already_true(
        &self,
        watch: &RoomTerminalWatch,
        after_offset: Option<u64>,
        now: Instant,
        program_running: bool,
    ) -> Option<RoomTerminalEventKind> {
        match watch {
            RoomTerminalWatch::Finished => {
                let end = self.last_program_end.as_ref()?;
                (end.output_offset > after_offset?).then(|| RoomTerminalEventKind::Finished {
                    exit_code: end.exit_code,
                    signal: end.signal.clone(),
                })
            }
            RoomTerminalWatch::OutputContains { text } => {
                let page = self.log.read(after_offset?, usize::MAX);
                if page.gap {
                    return None;
                }
                let mut plain = PlainText::new();
                plain.append(&page.bytes);
                plain
                    .since(0)
                    .contains(text.as_str())
                    .then(|| RoomTerminalEventKind::OutputMatched { text: text.clone() })
            }
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
