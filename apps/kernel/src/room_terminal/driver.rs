//! The one-driver rule, decided from the kernel's side of the PTY.
//!
//! At the prompt any Room agent can type. A foreground program (foreground
//! process group other than the shell) belongs to the actor who submitted the
//! command that started it until it exits. The user can always type.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::types::{RoomTerminalDriver, TerminalActor};

/// A command may take this long to become the foreground process group.
pub(super) const LAUNCH_SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct DriverTracker {
    last_submit: Option<(TerminalActor, Instant)>,
    program_owner: Option<TerminalActor>,
    /// Lines written to the terminal so far. The shell turns them into
    /// commands in this order, so the n-th line is normally the n-th command.
    lines_written: u64,
    /// The line number of each agent's latest line, to tell which command a
    /// wait of that agent is about.
    agent_lines: HashMap<String, u64>,
    last_write: Option<Instant>,
}

pub(super) enum Admission {
    Admit,
    Refuse(RoomTerminalDriver),
    /// A different actor just submitted a command that may not have started
    /// yet: look again shortly.
    Settle,
}

impl DriverTracker {
    /// A program the terminal was opened with belongs to its opener.
    pub(super) fn owned_by(actor: TerminalActor) -> Self {
        Self {
            last_submit: None,
            program_owner: Some(actor),
            ..Self::default()
        }
    }

    /// A command was just submitted at the prompt and may not be the
    /// foreground program yet.
    pub(super) fn launching(&self, now: Instant) -> bool {
        self.last_submit
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) < LAUNCH_SETTLE)
    }

    pub(super) fn observe(&mut self, shell_is_foreground: bool) -> RoomTerminalDriver {
        if shell_is_foreground {
            self.program_owner = None;
            return RoomTerminalDriver::Free;
        }
        let owner = self.program_owner.get_or_insert_with(|| {
            self.last_submit
                .as_ref()
                .map(|(actor, _)| actor.clone())
                .unwrap_or(TerminalActor::User)
        });
        RoomTerminalDriver::Busy {
            driven_by: owner.clone(),
        }
    }

    pub(super) fn admit(
        &mut self,
        actor: &TerminalActor,
        shell_is_foreground: bool,
        now: Instant,
    ) -> Admission {
        if *actor == TerminalActor::User {
            return Admission::Admit;
        }
        if shell_is_foreground {
            if let Some((submitter, at)) = &self.last_submit {
                if submitter != actor && now.duration_since(*at) < LAUNCH_SETTLE {
                    return Admission::Settle;
                }
            }
        }
        match self.observe(shell_is_foreground) {
            RoomTerminalDriver::Free => Admission::Admit,
            RoomTerminalDriver::Busy { driven_by } if driven_by == *actor => Admission::Admit,
            busy => Admission::Refuse(busy),
        }
    }

    /// The number of the latest line this agent wrote.
    pub(super) fn latest_line(&self, agent_id: &str) -> Option<u64> {
        self.agent_lines.get(agent_id).copied()
    }

    pub(super) fn lines_written(&self) -> u64 {
        self.lines_written
    }

    pub(super) fn last_write(&self) -> Option<Instant> {
        self.last_write
    }

    pub(super) fn record_write(
        &mut self,
        actor: &TerminalActor,
        shell_is_foreground: bool,
        input: &[u8],
        now: Instant,
    ) {
        self.last_write = Some(now);
        let lines = count_lines(input);
        if lines == 0 {
            return;
        }
        self.lines_written += lines;
        if let TerminalActor::Agent(agent_id) = actor {
            self.agent_lines
                .insert(agent_id.clone(), self.lines_written);
        }
        if shell_is_foreground {
            self.last_submit = Some((actor.clone(), now));
        }
    }

    /// Lines that never became commands (blank lines, answers a running
    /// command read, continuation lines) leave the count ahead of the shell:
    /// bring it back to the commands that really started.
    pub(super) fn settle_lines(&mut self, commands_started: u64) {
        self.lines_written = self.lines_written.min(commands_started);
        for line in self.agent_lines.values_mut() {
            *line = (*line).min(commands_started);
        }
    }
}

/// Line ends in `input`, counting CR LF as one.
fn count_lines(input: &[u8]) -> u64 {
    let mut lines = 0;
    let mut previous = 0;
    for &byte in input {
        match byte {
            b'\r' => lines += 1,
            b'\n' if previous != b'\r' => lines += 1,
            _ => {}
        }
        previous = byte;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> TerminalActor {
        TerminalActor::Agent(id.to_string())
    }

    #[test]
    fn foreground_program_belongs_to_the_agent_that_started_it() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        assert!(matches!(
            tracker.admit(&agent("a"), true, start),
            Admission::Admit
        ));
        tracker.record_write(&agent("a"), true, b"python3\n", start);
        let later = start + LAUNCH_SETTLE * 2;
        assert!(matches!(
            tracker.admit(&agent("b"), false, later),
            Admission::Refuse(RoomTerminalDriver::Busy { driven_by }) if driven_by == agent("a")
        ));
        assert!(matches!(
            tracker.admit(&agent("a"), false, later),
            Admission::Admit
        ));
        assert!(matches!(
            tracker.admit(&TerminalActor::User, false, later),
            Admission::Admit
        ));
        assert!(matches!(
            tracker.admit(&agent("b"), true, later),
            Admission::Admit
        ));
    }

    #[test]
    fn another_actor_waits_for_a_command_that_is_still_launching() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        tracker.record_write(&agent("a"), true, b"python3\r", start);
        assert!(matches!(
            tracker.admit(&agent("b"), true, start),
            Admission::Settle
        ));
    }

    #[test]
    fn a_program_the_user_started_is_driven_by_the_user() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        tracker.record_write(&TerminalActor::User, true, b"top\r", start);
        assert!(matches!(
            tracker.admit(&agent("a"), false, start + LAUNCH_SETTLE * 2),
            Admission::Refuse(RoomTerminalDriver::Busy {
                driven_by: TerminalActor::User
            })
        ));
    }

    #[test]
    fn lines_are_counted_by_their_ends() {
        assert_eq!(count_lines(b"ls"), 0);
        assert_eq!(count_lines(b"ls\r"), 1);
        assert_eq!(count_lines(b"a\r\nb\n\r"), 3);
    }

    #[test]
    fn an_agent_waits_for_the_command_of_its_latest_line() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        tracker.record_write(&agent("a"), true, b"sleep 2\r", start);
        tracker.record_write(&agent("b"), false, b"x\r", start);
        tracker.record_write(&agent("a"), false, b"true\r", start);
        assert_eq!(tracker.latest_line("a"), Some(3));
        assert_eq!(tracker.latest_line("b"), Some(2));
        tracker.settle_lines(1);
        assert_eq!(tracker.latest_line("a"), Some(1));
        assert_eq!(tracker.lines_written(), 1);
    }
}
