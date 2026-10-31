//! Who submitted the line that started a terminal's command. The submitter
//! decides whether the command may wait for room (a person's own never does).
//!
//! The shell runs the lines it is given one at a time and reports when each
//! starts and ends, so a line is tied to its commands by that order, never by
//! comparing clocks: lines accepted back to back, or a launch in the same
//! clock tick as the next submission, bind the same way, and no platform needs
//! a process start time.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Lines remembered; the shell reads them within moments of their submission.
const KEPT: usize = 256;
/// A background job is claimed within a sample or two of its launch.
const CLAIM_WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Submissions {
    /// For commands no submitted line explains: the shell's own start, a
    /// workload that has no terminal.
    default: bool,
    /// Lines accepted that the shell has not started, oldest first, and
    /// whether their commands may wait.
    queued: VecDeque<bool>,
    /// The line the shell is running now.
    running: Option<bool>,
    /// The last line the shell finished.
    finished: Option<bool>,
    /// The last job the shell reported as started in the background.
    last_background: Option<u32>,
    /// Background jobs of finished lines that no sample has claimed yet.
    unclaimed: VecDeque<(u32, bool, Instant)>,
}

impl Submissions {
    pub(super) fn new(default: bool) -> Self {
        Self {
            default,
            ..Self::default()
        }
    }

    pub(super) fn set_default(&mut self, holdable: bool) {
        self.default = holdable;
    }

    /// A line was accepted, to be read by the shell after those before it.
    pub(super) fn submit(&mut self, holdable: bool) {
        if self.queued.len() == KEPT {
            self.queued.pop_front();
        }
        self.queued.push_back(holdable);
    }

    /// The shell started the next line. One that nobody submitted through the
    /// terminal (a shell's own start, typed on a device) is the default's.
    pub(super) fn line_started(&mut self) {
        self.running = Some(self.queued.pop_front().unwrap_or(self.default));
    }

    /// The shell finished its line and reported the job it last put in the
    /// background (`$!`): that job belongs to the line when it is a new one.
    pub(super) fn line_finished(&mut self, background: Option<u32>) {
        let line = self.running.take();
        if let (Some(pid), Some(holdable)) = (background, line) {
            if self.last_background != Some(pid) {
                let now = Instant::now();
                self.unclaimed
                    .retain(|(_, _, at)| now.duration_since(*at) < CLAIM_WINDOW);
                if self.unclaimed.len() == KEPT {
                    self.unclaimed.pop_front();
                }
                self.unclaimed.push_back((pid, holdable, now));
            }
        }
        if background.is_some() {
            self.last_background = background;
        }
        self.finished = line.or(self.finished);
    }

    /// Whether the command whose top process is `pid`, first seen now, may
    /// wait: a job the shell named is its line's; otherwise it is what the
    /// running line launched, or, with the shell at its prompt, what the last
    /// line left behind.
    pub(super) fn claim(&mut self, pid: u32) -> bool {
        if let Some(at) = self.unclaimed.iter().position(|(job, _, _)| *job == pid) {
            return self
                .unclaimed
                .remove(at)
                .is_some_and(|(_, holdable, _)| holdable);
        }
        self.running.or(self.finished).unwrap_or(self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: bool = true;
    const PERSON: bool = false;

    /// The shell reads the next line, runs it and returns to the prompt.
    fn run_line(lines: &mut Submissions, background: Option<u32>) {
        lines.line_started();
        lines.line_finished(background);
    }

    #[test]
    fn a_job_follows_the_line_that_launched_it_when_the_next_line_is_accepted_first() {
        // The agent queues `build &`, the person queues `true` before the
        // shell reads either; the sampler sees the build after both ran.
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        lines.submit(PERSON);
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(500));
        assert!(
            lines.claim(500),
            "the person's `true` took the agent's build"
        );
    }

    #[test]
    fn the_persons_job_stays_the_persons_when_the_agent_queues_after_it() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(PERSON);
        lines.submit(AGENT);
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(500));
        assert!(!lines.claim(500), "the agent's line took the person's job");
    }

    #[test]
    fn each_of_two_jobs_keeps_its_own_line_whatever_order_they_are_sampled_in() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        lines.submit(PERSON);
        lines.submit(AGENT);
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(501));
        run_line(&mut lines, Some(502));
        assert!(!lines.claim(501));
        assert!(lines.claim(502));
        assert!(lines.claim(500));
    }

    #[test]
    fn a_command_seen_while_its_line_runs_belongs_to_that_line() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        lines.submit(PERSON);
        lines.line_started();
        assert!(lines.claim(700), "the agent's foreground build");
        lines.line_finished(None);
        lines.line_started();
        assert!(!lines.claim(701), "the person's foreground command");
    }

    #[test]
    fn a_command_found_at_the_prompt_belongs_to_the_last_line() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        run_line(&mut lines, None);
        assert!(lines.claim(800));
    }

    #[test]
    fn a_job_is_claimed_once() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        run_line(&mut lines, Some(500));
        run_line(&mut lines, None);
        assert!(lines.claim(500));
        // The same number again is another process: the last line's.
        lines.submit(PERSON);
        run_line(&mut lines, None);
        assert!(!lines.claim(500));
    }

    #[test]
    fn a_line_nobody_submitted_and_a_fresh_terminal_follow_the_default() {
        let mut lines = Submissions::new(PERSON);
        assert!(!lines.claim(900), "before any line");
        lines.submit(AGENT);
        run_line(&mut lines, Some(500));
        lines.line_started();
        assert!(!lines.claim(901), "a line that was not submitted here");
    }

    #[test]
    fn a_background_pid_the_shell_repeats_is_not_a_new_job() {
        // `$!` stays the last job's after later lines: only a change is a launch.
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT);
        lines.submit(PERSON);
        run_line(&mut lines, Some(500));
        assert!(lines.claim(500));
        run_line(&mut lines, Some(500));
        assert!(
            !lines.claim(500),
            "a repeat of `$!` was claimed as the person's job"
        );
    }
}
