//! One tick of a running terminal: PTY output into the screen, shell exit,
//! foreground changes and the watches they fire.

use std::time::{Duration, Instant};

use super::events::{Fired, QUIET_BEFORE_WAITING};
use super::terminal::{ProgramEnd, Terminal};
use super::types::RoomTerminalOutcome;
use crate::pty::{PtyManager, PtyProcessState};

/// How long an idle shell may take to start a command that was queued as
/// typeahead while the previous one still ran.
const UNRUN_LINES_SETTLE: Duration = Duration::from_millis(400);

pub(super) struct Pumped {
    pub output_changed: bool,
    pub fired: Fired,
}

impl Terminal {
    /// Only what the PTY printed. Cheap enough to run every few milliseconds
    /// while someone types; sampling the foreground program at that rate would
    /// count the shell's own start-up helpers as programs.
    pub(super) fn pump_output(&mut self, pty: &mut PtyManager, now: Instant) -> Pumped {
        let mut pumped = Pumped {
            output_changed: false,
            fired: Fired::new(),
        };
        if let Some(run_id) = self.run_id.clone() {
            pumped.output_changed = self.drain(pty, &run_id, now, &mut pumped.fired);
        }
        pumped
    }

    pub(super) fn pump(&mut self, pty: &mut PtyManager, now: Instant) -> Pumped {
        let mut pumped = Pumped {
            output_changed: false,
            fired: Fired::new(),
        };
        let Some(run_id) = self.run_id.clone() else {
            return pumped;
        };
        if self.drain(pty, &run_id, now, &mut pumped.fired) {
            pumped.output_changed = true;
        }
        if let Ok(PtyProcessState::Exited { exit_code, signal }) = pty.poll_process_state(&run_id) {
            self.drain(pty, &run_id, now, &mut pumped.fired);
            if let Some(sweep) = &self.session_sweep {
                sweep.stop();
            }
            let _ = pty.remove_process(&run_id);
            self.record_program_end(ProgramEnd {
                output_offset: self.log.end_offset(),
                command: None,
                at: now,
                exit_code,
                signal: signal.clone(),
            });
            pumped
                .fired
                .extend(self.watches.on_shell_exit(exit_code, signal.clone()));
            self.end(RoomTerminalOutcome::ProgramExited { exit_code, signal });
            pumped.output_changed = true;
            return pumped;
        }
        // The login files' programs are nobody's commands: they get no number
        // and end no wait.
        let program_running = !self.shell_is_foreground(pty) && !self.starting(now);
        // Once the shell reports its commands, sampling would only race it.
        if !self.mark_tracking {
            let mut ended_command = None;
            if program_running && !self.program_running && !self.command_in_flight {
                self.commands_started += 1;
            }
            if self.program_running && !program_running {
                self.command_in_flight = false;
                ended_command = Some(self.commands_started);
                self.record_program_end(ProgramEnd {
                    output_offset: self.log.end_offset(),
                    command: ended_command,
                    at: now,
                    exit_code: None,
                    signal: None,
                });
            }
            // A start mark alone is not an observed foreground program.
            // Only a real foreground transition can complete the fallback.
            if program_running || self.program_running {
                pumped
                    .fired
                    .extend(self.watches.on_foreground(program_running, ended_command));
            }
        }
        self.program_running = program_running;
        pumped.fired.extend(self.settle_unrun_lines(pty, now));
        if !self.at_prompt(pty, now)
            && !self.waiting_reported
            && now.duration_since(self.last_output_at) >= QUIET_BEFORE_WAITING
            && self.watches.is_waiting_for_input()
        {
            self.waiting_reported = true;
            pumped.fired.extend(self.watches.on_quiet_program());
        }
        pumped
    }

    /// Lines that never became commands leave the shell idle with the line
    /// count ahead of the commands. Once it has been quiet for a while no
    /// queued command is left to start, so waits stop expecting one.
    fn settle_unrun_lines(&mut self, pty: &PtyManager, now: Instant) -> Fired {
        // A line typed during the login files waits for the first prompt.
        if self.is_busy(pty)
            || self.starting(now)
            || self.driver.lines_written() <= self.commands_started
        {
            return Fired::new();
        }
        let quiet_since = [
            self.driver.last_write(),
            self.last_program_end.as_ref().map(|end| end.at),
        ]
        .into_iter()
        .flatten()
        .max();
        if quiet_since.is_some_and(|since| now.duration_since(since) < UNRUN_LINES_SETTLE) {
            return Fired::new();
        }
        self.driver.settle_lines(self.commands_started);
        self.watches
            .on_lines_settled(self.commands_started, self.last_program_end.as_ref())
    }

    fn drain(
        &mut self,
        pty: &mut PtyManager,
        run_id: &str,
        now: Instant,
        fired: &mut Fired,
    ) -> bool {
        let mut produced = false;
        for chunk in pty.drain_output(run_id).unwrap_or_default() {
            fired.extend(self.apply_output(&chunk.bytes, now));
            produced = true;
        }
        produced
    }
}
