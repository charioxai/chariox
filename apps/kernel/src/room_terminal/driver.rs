//! The one-driver rule, decided from the kernel's side of the PTY.
//!
//! A submitted line belongs to its submitter while it waits for the shell and
//! while its program runs. The user can always type.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use super::types::{RoomTerminalDriver, TerminalActor};

struct Submission {
    actor: TerminalActor,
    lines: usize,
}

#[derive(Default)]
pub(super) struct DriverTracker {
    submissions: VecDeque<Submission>,
    opener: Option<TerminalActor>,
    /// Lines written to the terminal so far. The shell turns them into
    /// commands in this order, so the n-th line is normally the n-th command.
    lines_written: u64,
    /// The line number of each agent's latest line, to tell which command a
    /// wait of that agent is about.
    agent_lines: HashMap<String, u64>,
    last_write: Option<Instant>,
    last_submission: Option<Instant>,
    last_finish_report: Option<Instant>,
}

impl DriverTracker {
    /// A program the terminal was opened with belongs to its opener.
    pub(super) fn owned_by(actor: TerminalActor) -> Self {
        Self {
            opener: Some(actor),
            ..Self::default()
        }
    }

    /// A command was just submitted at the prompt and may not be the
    /// foreground program yet.
    pub(super) fn has_submission(&self) -> bool {
        !self.submissions.is_empty()
    }

    pub(super) fn observe(&self, at_prompt: bool) -> RoomTerminalDriver {
        if let Some(submission) = self.submissions.front() {
            return RoomTerminalDriver::Busy {
                driven_by: submission.actor.clone(),
            };
        }
        if at_prompt {
            return RoomTerminalDriver::Free;
        }
        RoomTerminalDriver::Busy {
            driven_by: self.opener.clone().unwrap_or(TerminalActor::User),
        }
    }

    pub(super) fn finish_submission(&mut self, now: Instant) {
        self.last_finish_report = Some(now);
        if let Some(submission) = self.submissions.front_mut() {
            submission.lines -= 1;
            if submission.lines == 0 {
                self.submissions.pop_front();
            }
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

    pub(super) fn finish_since_last_submission(&self) -> Option<Instant> {
        let submitted = self.last_submission?;
        self.last_finish_report
            .filter(|finished| *finished >= submitted)
    }

    pub(super) fn record_write(
        &mut self,
        actor: &TerminalActor,
        submitted_lines: usize,
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
        if submitted_lines > 0 {
            self.last_submission = Some(now);
            match self.submissions.back_mut() {
                Some(submission) if submission.actor == *actor => {
                    submission.lines += submitted_lines;
                }
                _ => self.submissions.push_back(Submission {
                    actor: actor.clone(),
                    lines: submitted_lines,
                }),
            }
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

    pub(super) fn discard_submissions(&mut self, keep_running: bool) {
        self.submissions.truncate(usize::from(keep_running));
        if let Some(submission) = self.submissions.front_mut() {
            submission.lines = 1;
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
    fn queued_lines_keep_their_submitter_until_each_line_finishes() {
        let mut tracker = DriverTracker::default();
        let now = Instant::now();
        tracker.record_write(&agent("a"), 2, b"first\rsecond\r", now);
        tracker.record_write(&TerminalActor::User, 1, b"third\r", now);

        let agent_driving = RoomTerminalDriver::Busy {
            driven_by: agent("a"),
        };
        assert_eq!(tracker.observe(true), agent_driving);
        assert_eq!(tracker.observe(false), agent_driving);
        tracker.finish_submission(now);
        assert_eq!(tracker.observe(true), agent_driving);
        tracker.finish_submission(now);
        assert_eq!(
            tracker.observe(true),
            RoomTerminalDriver::Busy {
                driven_by: TerminalActor::User
            }
        );
        tracker.finish_submission(now);
        assert_eq!(tracker.observe(true), RoomTerminalDriver::Free);
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
        tracker.record_write(&agent("a"), 1, b"sleep 2\r", start);
        tracker.record_write(&agent("b"), 0, b"x\r", start);
        tracker.record_write(&agent("a"), 0, b"true\r", start);
        assert_eq!(tracker.latest_line("a"), Some(3));
        assert_eq!(tracker.latest_line("b"), Some(2));
        tracker.settle_lines(1);
        assert_eq!(tracker.latest_line("a"), Some(1));
        assert_eq!(tracker.lines_written(), 1);
    }
}
