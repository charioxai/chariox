//! Who submitted the line that started a terminal's command. The submitter
//! decides whether the command may wait for room (a person's own never does),
//! and a line is bound to the commands it starts by when it was submitted and
//! when they started, not by who typed last when the sampler first sees them.
use std::collections::VecDeque;

/// Lines remembered; a command is claimed within a sample or two of starting.
const KEPT: usize = 256;

#[derive(Default)]
pub(super) struct Submissions {
    /// For commands no submitted line explains: the shell's own start, a
    /// workload that has no terminal.
    default: bool,
    /// Submitted lines, oldest first: when (clock ticks since boot) and
    /// whether their commands may wait.
    lines: VecDeque<(u64, bool)>,
}

impl Submissions {
    pub(super) fn new(default: bool) -> Self {
        Self {
            default,
            lines: VecDeque::new(),
        }
    }

    pub(super) fn set_default(&mut self, holdable: bool) {
        self.default = holdable;
    }

    pub(super) fn record(&mut self, at: u64, holdable: bool) {
        if self.lines.len() == KEPT {
            self.lines.pop_front();
        }
        self.lines.push_back((at, holdable));
    }

    /// What a command that starts now gets.
    pub(super) fn latest(&self) -> bool {
        self.lines
            .back()
            .map_or(self.default, |(_, holdable)| *holdable)
    }

    /// Whether the command that started at `started` may wait: the policy of
    /// the last line submitted before it started, whoever typed since. A
    /// command whose start the platform does not report (or that is gone
    /// already) gets the latest line's.
    pub(super) fn owner_of(&self, started: Option<u64>) -> bool {
        let Some(started) = started else {
            return self.latest();
        };
        self.lines
            .iter()
            .rev()
            .find(|(at, _)| *at <= started)
            .map_or(self.default, |(_, holdable)| *holdable)
    }
}

/// Clock ticks since boot, the unit a process's start time is read in.
#[cfg(target_os = "linux")]
pub(super) fn now() -> u64 {
    let mut at = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the pointer names one timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut at) };
    // SAFETY: sysconf reads a constant.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    (at.tv_sec as u64) * ticks + (at.tv_nsec as u64) * ticks / 1_000_000_000
}

#[cfg(not(target_os = "linux"))]
pub(super) fn now() -> u64 {
    0
}
