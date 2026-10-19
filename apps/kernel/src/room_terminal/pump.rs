//! One tick of a running terminal: PTY output into the screen, shell exit,
//! foreground changes and the watches they fire.

use std::time::Instant;

use super::events::{Fired, QUIET_BEFORE_WAITING};
use super::terminal::{ProgramEnd, Terminal};
use super::types::RoomTerminalOutcome;
use crate::pty::{PtyManager, PtyProcessState};

pub(super) struct Pumped {
    pub output_changed: bool,
    pub fired: Fired,
}

impl Terminal {
    pub(super) fn pump(&mut self, pty: &mut PtyManager, now: Instant) -> Pumped {
        let mut pumped = Pumped {
            output_changed: false,
            fired: Fired::new(),
        };
        let Some(run_id) = self.run_id.clone() else {
            return pumped;
        };
        if self.drain(pty, &run_id, now) {
            pumped.output_changed = true;
            pumped.fired.extend(self.watches.on_output(&self.plain));
        }
        if let Ok(PtyProcessState::Exited { exit_code, signal }) = pty.poll_process_state(&run_id)
        {
            if self.drain(pty, &run_id, now) {
                pumped.fired.extend(self.watches.on_output(&self.plain));
            }
            let _ = pty.remove_process(&run_id);
            self.last_program_end = Some(ProgramEnd {
                output_offset: self.log.end_offset(),
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
        let program_running = !self.shell_is_foreground(pty);
        if self.program_running && !program_running {
            self.last_program_end = Some(ProgramEnd {
                output_offset: self.log.end_offset(),
                exit_code: None,
                signal: None,
            });
        }
        self.program_running = program_running;
        pumped.fired.extend(self.watches.on_foreground(program_running));
        if program_running
            && !self.waiting_reported
            && now.duration_since(self.last_output_at) >= QUIET_BEFORE_WAITING
            && self.watches.is_waiting_for_input()
        {
            self.waiting_reported = true;
            pumped.fired.extend(self.watches.on_quiet_program());
        }
        pumped
    }

    fn drain(&mut self, pty: &mut PtyManager, run_id: &str, now: Instant) -> bool {
        let mut produced = false;
        for chunk in pty.drain_output(run_id).unwrap_or_default() {
            self.apply_output(&chunk.bytes, now);
            produced = true;
        }
        produced
    }
}
