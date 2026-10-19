//! One terminal's state: its PTY handle, screen, output and driver.

use std::time::Instant;

use base64::Engine as _;

use super::driver::DriverTracker;
use super::output_log::OutputLog;
use super::plain_text::PlainText;
use super::screen::Screen;
use super::store::TerminalRecord;
use super::types::{
    RoomTerminalDriver, RoomTerminalOutcome, RoomTerminalSnapshot, RoomTerminalStatus,
    RoomTerminalSummary, TerminalActor,
};
use super::watch::WatchSet;

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
}

impl Terminal {
    pub(super) fn running(
        record: TerminalRecord,
        run_id: String,
        shell_pid: Option<i32>,
    ) -> Self {
        let screen = Screen::new(record.rows, record.cols);
        Self::new(record, Some(run_id), shell_pid, screen)
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

    pub(super) fn end(&mut self, outcome: RoomTerminalOutcome) {
        self.run_id = None;
        self.record.ended = Some(outcome);
        self.unsaved_since = Some(Instant::now());
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
    pub(super) fn to_record(&mut self) -> TerminalRecord {
        let text = self.screen.text();
        TerminalRecord {
            scrollback_text: text.scrollback,
            screen_text: text.screen,
            ..self.record.clone()
        }
    }

    pub(super) fn opened_by(&self) -> &TerminalActor {
        &self.record.opened_by
    }
}
