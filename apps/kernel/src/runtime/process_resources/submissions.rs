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

/// Lines remembered.
const KEPT: usize = 256;
/// The shell reads an accepted line within moments. One it has not reported
/// after this long was read by a program, or the shell reports nothing: it no
/// longer holds the ones behind it back.
const LINE_WINDOW: Duration = Duration::from_secs(10);
/// A background job is claimed within a sample or two of its launch.
const CLAIM_WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Submissions {
    /// For commands no submitted line explains: the shell's own start, a
    /// workload that has no terminal.
    default: bool,
    /// Lines accepted that the shell has not reported starting, oldest
    /// first: whether their commands may wait, and when they were accepted.
    queued: VecDeque<(bool, Instant)>,
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

    fn expire(&mut self, now: Instant) {
        self.queued
            .retain(|(_, at)| now.saturating_duration_since(*at) < LINE_WINDOW);
    }

    /// A line was accepted, to be read by the shell after those before it.
    pub(super) fn submit(&mut self, holdable: bool, now: Instant) {
        self.expire(now);
        if self.queued.len() == KEPT {
            self.queued.pop_front();
        }
        self.queued.push_back((holdable, now));
    }

    /// The shell started the next line. One that nobody submitted through the
    /// terminal (a shell's own start, typed on a device) is the default's.
    pub(super) fn line_started(&mut self, now: Instant) {
        self.expire(now);
        self.running = Some(
            self.queued
                .pop_front()
                .map_or(self.default, |(holdable, _)| holdable),
        );
    }

    /// The shell finished its line and reported the job it last put in the
    /// background (`$!`): that job belongs to the line when it is a new one.
    pub(super) fn line_finished(&mut self, background: Option<u32>, now: Instant) {
        let line = self.running.take();
        if let (Some(pid), Some(holdable)) = (background, line) {
            if self.last_background != Some(pid) {
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
    /// line left behind. While an accepted line is still unreported the
    /// shell's state is not known yet and there is no answer: the command is
    /// asked about again at the next sample.
    pub(super) fn claim(&mut self, pid: u32, now: Instant) -> Option<bool> {
        if let Some(at) = self.unclaimed.iter().position(|(job, _, _)| *job == pid) {
            return self.unclaimed.remove(at).map(|(_, holdable, _)| holdable);
        }
        self.expire(now);
        if !self.queued.is_empty() {
            return None;
        }
        Some(self.running.or(self.finished).unwrap_or(self.default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Instant {
        Instant::now()
    }

    fn holdable(lines: &mut Submissions, pid: u32) -> bool {
        lines.claim(pid, t()).expect("the shell's state is known")
    }

    const AGENT: bool = true;
    const PERSON: bool = false;

    /// The shell reads the next line, runs it and returns to the prompt.
    fn run_line(lines: &mut Submissions, background: Option<u32>) {
        lines.line_started(t());
        lines.line_finished(background, t());
    }

    #[test]
    fn a_job_follows_the_line_that_launched_it_when_the_next_line_is_accepted_first() {
        // The agent queues `build &`, the person queues `true` before the
        // shell reads either; the sampler sees the build after both ran.
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(500));
        assert!(
            holdable(&mut lines, 500),
            "the person's `true` took the agent's build"
        );
    }

    #[test]
    fn the_persons_job_stays_the_persons_when_the_agent_queues_after_it() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(PERSON, t());
        lines.submit(AGENT, t());
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(500));
        assert!(
            !holdable(&mut lines, 500),
            "the agent's line took the person's job"
        );
    }

    #[test]
    fn each_of_two_jobs_keeps_its_own_line_whatever_order_they_are_sampled_in() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        lines.submit(AGENT, t());
        run_line(&mut lines, Some(500));
        run_line(&mut lines, Some(501));
        run_line(&mut lines, Some(502));
        assert!(!holdable(&mut lines, 501));
        assert!(holdable(&mut lines, 502));
        assert!(holdable(&mut lines, 500));
    }

    #[test]
    fn a_command_seen_while_its_line_runs_belongs_to_that_line() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        lines.line_started(t());
        assert!(holdable(&mut lines, 700), "the agent's foreground build");
        lines.line_finished(None, t());
        lines.submit(PERSON, t());
        lines.line_started(t());
        assert!(
            !holdable(&mut lines, 701),
            "the person's foreground command"
        );
    }

    #[test]
    fn a_command_found_at_the_prompt_belongs_to_the_last_line() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        run_line(&mut lines, None);
        assert!(holdable(&mut lines, 800));
    }

    #[test]
    fn a_job_is_claimed_once() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        run_line(&mut lines, Some(500));
        run_line(&mut lines, None);
        assert!(holdable(&mut lines, 500));
        // The same number again is another process: the last line's.
        lines.submit(PERSON, t());
        run_line(&mut lines, None);
        assert!(!holdable(&mut lines, 500));
    }

    #[test]
    fn a_line_nobody_submitted_and_a_fresh_terminal_follow_the_default() {
        let mut lines = Submissions::new(PERSON);
        assert!(!holdable(&mut lines, 900), "before any line");
        lines.submit(AGENT, t());
        run_line(&mut lines, Some(500));
        lines.line_started(t());
        assert!(
            !holdable(&mut lines, 901),
            "a line that was not submitted here"
        );
    }

    #[test]
    fn a_background_pid_the_shell_repeats_is_not_a_new_job() {
        // `$!` stays the last job's after later lines: only a change is a launch.
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        run_line(&mut lines, Some(500));
        assert!(holdable(&mut lines, 500));
        run_line(&mut lines, Some(500));
        assert!(
            !holdable(&mut lines, 500),
            "a repeat of `$!` was claimed as the person's job"
        );
    }

    #[test]
    fn a_command_is_not_decided_while_an_accepted_line_is_unreported() {
        // The sampler can see a launch before the shell's start mark is read.
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        assert_eq!(lines.claim(900, t()), None);
        lines.line_started(t());
        assert_eq!(lines.claim(900, t()), Some(AGENT));
    }

    #[test]
    fn a_named_job_is_decided_whatever_is_queued_behind_it() {
        let mut lines = Submissions::new(PERSON);
        lines.submit(AGENT, t());
        run_line(&mut lines, Some(500));
        lines.submit(PERSON, t());
        assert_eq!(lines.claim(500, t()), Some(AGENT));
    }

    #[test]
    fn a_line_the_shell_never_reports_stops_holding_commands_back() {
        // Read by a program, or a shell that reports nothing.
        let mut lines = Submissions::new(PERSON);
        let accepted = t();
        lines.submit(AGENT, accepted);
        assert_eq!(lines.claim(900, accepted), None);
        let later = accepted + LINE_WINDOW;
        assert_eq!(lines.claim(900, later), Some(PERSON));
        lines.submit(AGENT, later);
        lines.line_started(later);
        assert!(
            lines.claim(901, later) == Some(AGENT),
            "the next line is not shifted"
        );
    }
}
