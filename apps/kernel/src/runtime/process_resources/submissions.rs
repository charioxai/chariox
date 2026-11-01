//! Who launched a terminal's command: the actor whose Enter submitted the line
//! that started it. That decides whether the command may wait for room (a
//! person's own never does).
//!
//! The shell reports each line it runs, one at a time (OSC 133 `C` when it
//! starts, `D` when it ends, with the process groups of its job table), and
//! prints exactly one `D` per Enter at its prompt: a line that runs, or an
//! empty one. So Enters and reports pair up in order and nothing is compared
//! against a clock or a process start time.
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// Lines remembered.
const KEPT: usize = 256;
/// The shell reads an accepted line within moments. One it has not reported
/// after this long was read by a program, or continued on another prompt: it
/// no longer holds the ones behind it back.
const LINE_WINDOW: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Submissions {
    /// For commands no submitted line explains: the shell's own start, a
    /// workload that has no terminal.
    default: bool,
    /// Without a shell that reports its lines (fish, anything but bash and
    /// zsh) the terminal belongs to whoever submitted last.
    unreported: bool,
    last: Option<bool>,
    /// Enters accepted that the shell has not reported yet, oldest first:
    /// whether their commands may wait, and when they were accepted.
    queued: VecDeque<(bool, Instant)>,
    /// The shell's first prompt is reported before any line was entered.
    prompted: bool,
    /// The line the shell is running now.
    running: Option<bool>,
    /// The last line the shell ran to its end.
    finished: Option<bool>,
    /// The shell's job table: each process group and the line that started it.
    jobs: BTreeMap<u32, bool>,
}

impl Submissions {
    pub(super) fn new(default: bool) -> Self {
        Self {
            default,
            ..Self::default()
        }
    }

    /// A terminal's own start is a person's. `reports_lines`: the shell sends
    /// marks.
    pub(super) fn for_terminal(&mut self, reports_lines: bool) {
        self.default = false;
        self.unreported = !reports_lines;
    }

    fn expire(&mut self, now: Instant) {
        self.queued
            .retain(|(_, at)| now.saturating_duration_since(*at) < LINE_WINDOW);
    }

    /// An Enter was accepted at the prompt, to be read by the shell after
    /// those before it.
    pub(super) fn submit(&mut self, holdable: bool, now: Instant) {
        self.last = Some(holdable);
        if self.unreported {
            return;
        }
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

    /// The shell is back at its prompt. It lists the process groups it still
    /// holds: those new here were started by the line that just ended.
    pub(super) fn line_finished(&mut self, jobs: &[u32]) {
        let ran = self.running.take();
        // An Enter on an empty line reports no start: it is its own report.
        if ran.is_none() && self.prompted {
            self.queued.pop_front();
        }
        self.prompted = true;
        let line = ran.or(self.finished).unwrap_or(self.default);
        self.jobs = jobs
            .iter()
            .map(|group| (*group, self.jobs.get(group).copied().unwrap_or(line)))
            .collect();
        self.finished = ran.or(self.finished);
    }

    /// Whether the command whose top process is `pid`, in process group
    /// `group`, first seen now, may wait: a job of the shell's table is its
    /// line's; otherwise it is what the running line launched, or, with the
    /// shell at its prompt, what the last line left behind. While the shell
    /// reports no line running but has accepted ones it has not reported, its
    /// state is not known yet and there is no answer: the command is asked
    /// about again at the next sample.
    pub(super) fn claim(&mut self, pid: u32, group: Option<u32>, now: Instant) -> Option<bool> {
        if self.unreported {
            return Some(self.last.unwrap_or(self.default));
        }
        if let Some(holdable) = group
            .and_then(|group| self.jobs.get(&group))
            .or_else(|| self.jobs.get(&pid))
        {
            return Some(*holdable);
        }
        if self.running.is_some() {
            return self.running;
        }
        self.expire(now);
        if !self.queued.is_empty() {
            return None;
        }
        Some(self.finished.unwrap_or(self.default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: bool = true;
    const PERSON: bool = false;

    fn t() -> Instant {
        Instant::now()
    }

    fn holdable(lines: &mut Submissions, pid: u32) -> bool {
        lines
            .claim(pid, Some(pid), t())
            .expect("the shell's state is known")
    }

    /// The shell reads the next line, runs it and returns to the prompt,
    /// holding `jobs`.
    fn run_line(lines: &mut Submissions, jobs: &[u32]) {
        lines.line_started(t());
        lines.line_finished(jobs);
    }

    /// A shell at its first prompt.
    fn terminal() -> Submissions {
        let mut lines = Submissions::new(PERSON);
        lines.for_terminal(true);
        lines.line_finished(&[]);
        lines
    }

    #[test]
    fn a_job_follows_the_line_that_launched_it_when_the_next_line_is_accepted_first() {
        let mut lines = terminal();
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        run_line(&mut lines, &[500]);
        run_line(&mut lines, &[500]);
        assert!(
            holdable(&mut lines, 500),
            "the person's `true` took the agent's build"
        );
    }

    #[test]
    fn the_persons_job_stays_the_persons_when_the_agent_queues_after_it() {
        let mut lines = terminal();
        lines.submit(PERSON, t());
        lines.submit(AGENT, t());
        run_line(&mut lines, &[500]);
        run_line(&mut lines, &[500]);
        assert!(
            !holdable(&mut lines, 500),
            "the agent's line took the person's job"
        );
    }

    #[test]
    fn every_job_a_line_starts_follows_it_and_each_pipeline_member_resolves_by_group() {
        let mut lines = terminal();
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        run_line(&mut lines, &[500, 600]);
        run_line(&mut lines, &[500, 600, 700]);
        assert!(holdable(&mut lines, 500));
        assert!(holdable(&mut lines, 600));
        assert!(!holdable(&mut lines, 700));
        // A pipeline's later member has its own pid, the group of the first.
        assert_eq!(lines.claim(601, Some(600), t()), Some(AGENT));
    }

    #[test]
    fn a_job_keeps_its_line_until_the_shell_stops_listing_its_group() {
        let mut lines = terminal();
        lines.submit(AGENT, t());
        run_line(&mut lines, &[500]);
        lines.submit(PERSON, t());
        run_line(&mut lines, &[500]);
        assert!(holdable(&mut lines, 500));
        lines.submit(PERSON, t());
        run_line(&mut lines, &[]);
        // The number comes back as another process group.
        lines.submit(PERSON, t());
        run_line(&mut lines, &[500]);
        assert!(!holdable(&mut lines, 500));
    }

    #[test]
    fn a_running_command_is_decided_whatever_is_queued_behind_its_line() {
        let mut lines = terminal();
        lines.submit(AGENT, t());
        lines.submit(PERSON, t());
        lines.line_started(t());
        assert_eq!(lines.claim(700, Some(700), t()), Some(AGENT));
    }

    #[test]
    fn a_command_at_the_prompt_belongs_to_the_last_line() {
        let mut lines = terminal();
        lines.submit(AGENT, t());
        run_line(&mut lines, &[]);
        assert!(holdable(&mut lines, 800));
    }

    #[test]
    fn an_empty_line_consumes_its_own_enter() {
        let mut lines = terminal();
        lines.submit(PERSON, t());
        lines.line_finished(&[]);
        lines.submit(AGENT, t());
        lines.line_started(t());
        assert_eq!(
            lines.claim(900, Some(900), t()),
            Some(AGENT),
            "the empty Enter shifted the next line"
        );
    }

    #[test]
    fn the_first_prompt_consumes_nothing() {
        let mut lines = Submissions::new(PERSON);
        lines.for_terminal(true);
        lines.submit(AGENT, t());
        lines.line_finished(&[]);
        lines.line_started(t());
        assert_eq!(lines.claim(900, Some(900), t()), Some(AGENT));
    }

    #[test]
    fn a_line_nobody_submitted_and_a_fresh_terminal_follow_the_default() {
        let mut lines = terminal();
        assert!(!holdable(&mut lines, 900), "before any line");
        lines.line_started(t());
        assert!(
            !holdable(&mut lines, 901),
            "a line that was not submitted here"
        );
    }

    #[test]
    fn a_command_is_not_decided_while_an_accepted_line_is_unreported() {
        // The sampler can see a launch before the shell's start mark is read.
        let mut lines = terminal();
        lines.submit(AGENT, t());
        assert_eq!(lines.claim(900, Some(900), t()), None);
        lines.line_started(t());
        assert_eq!(lines.claim(900, Some(900), t()), Some(AGENT));
    }

    #[test]
    fn a_line_the_shell_never_reports_stops_holding_commands_back() {
        // Read by a program, or continued on another prompt.
        let mut lines = terminal();
        let accepted = t();
        lines.submit(AGENT, accepted);
        assert_eq!(lines.claim(900, Some(900), accepted), None);
        let later = accepted + LINE_WINDOW;
        assert_eq!(lines.claim(900, Some(900), later), Some(PERSON));
    }

    #[test]
    fn without_marks_the_latest_submitter_owns_the_terminal() {
        let mut lines = Submissions::new(AGENT);
        lines.for_terminal(false);
        assert_eq!(
            lines.claim(900, None, t()),
            Some(PERSON),
            "nobody submitted yet"
        );
        lines.submit(AGENT, t());
        assert_eq!(lines.claim(901, None, t()), Some(AGENT));
        lines.submit(PERSON, t());
        assert_eq!(lines.claim(902, None, t()), Some(PERSON));
    }
}
