//! What an agent reads in its inbox about its own command that waits for room:
//! one short message when it is paused or held, one when it continues or is
//! ended. The messages sit in the inbox for the agent to read when it looks;
//! they never start a turn or steer one. Flips within a few seconds are told
//! once, by the state they settle in.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::runtime::process_resources::{
    SessionProcessState, WaitChange, WaitEvent, WaitReason, Waiting,
};

/// A state that holds this long is told; a flip back inside it is not.
const SETTLE: Duration = Duration::from_secs(3);
/// Commands tracked at once; the least recently changed go first.
const TRACKED: usize = 256;
const COMMAND_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    Waiting,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Notice {
    pub session_id: String,
    pub agent_id: String,
    pub process_id: String,
    pub command: String,
    pub resource: &'static str,
    pub state: &'static str,
    pub message: String,
}

struct Tracked {
    session_id: String,
    agent_id: String,
    command: String,
    resource: WaitReason,
    held: bool,
    want: Want,
    told_waiting: bool,
    changed_at: Instant,
}

#[derive(Default)]
pub(super) struct WaitNotices {
    tracked: BTreeMap<String, Tracked>,
}

impl WaitNotices {
    /// Takes one wait change of a command that belongs to an agent. An end is
    /// told at once; the other changes wait to settle.
    pub(super) fn record(
        &mut self,
        session_id: &str,
        agent_id: &str,
        event: &WaitEvent,
        now: Instant,
    ) -> Option<Notice> {
        let row = match &event.state {
            SessionProcessState::Paused(row) | SessionProcessState::Held(row) => Some(row),
            SessionProcessState::Running => None,
        };
        match &event.change {
            WaitChange::Began => {
                let row = row?;
                let want_changed = self
                    .tracked
                    .get(&event.process_id)
                    .is_none_or(|tracked| tracked.want != Want::Waiting);
                let tracked = self
                    .tracked
                    .entry(event.process_id.clone())
                    .or_insert_with(|| Tracked::new(session_id, agent_id, row, now));
                tracked.held = matches!(event.state, SessionProcessState::Held(_));
                tracked.want = Want::Waiting;
                if want_changed {
                    tracked.changed_at = now;
                }
                self.forget_oldest();
                None
            }
            WaitChange::Resumed { .. } => {
                let tracked = self.tracked.get_mut(&event.process_id)?;
                if tracked.want != Want::Running {
                    tracked.want = Want::Running;
                    tracked.changed_at = now;
                }
                None
            }
            WaitChange::Stopped { .. } => {
                let tracked = self.tracked.remove(&event.process_id);
                let row = row?;
                let held = matches!(event.state, SessionProcessState::Held(_));
                let command = tracked.map_or_else(|| short(&row.command_line), |t| t.command);
                let waited = crate::session::unix_epoch_ms().saturating_sub(row.since_ms);
                Some(Notice {
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                    process_id: event.process_id.clone(),
                    resource: noun(row.reason),
                    state: "ended",
                    message: format!(
                        "`{command}` was ended after {} {}: nothing freed {}. Run it again.",
                        span(waited),
                        if held { "waiting" } else { "paused" },
                        noun(row.reason),
                    ),
                    command,
                })
            }
        }
    }

    /// The notices whose state has held for the settle time.
    pub(super) fn due(&mut self, now: Instant) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.tracked.retain(|process_id, tracked| {
            if now.saturating_duration_since(tracked.changed_at) < SETTLE {
                return true;
            }
            match (tracked.want, tracked.told_waiting) {
                (Want::Waiting, false) => {
                    tracked.told_waiting = true;
                    notices.push(tracked.notice(process_id, "waiting"));
                    true
                }
                (Want::Waiting, true) => true,
                (Want::Running, true) => {
                    notices.push(tracked.notice(process_id, "running"));
                    false
                }
                // It was never told as waiting: nothing to take back.
                (Want::Running, false) => false,
            }
        });
        notices
    }

    fn forget_oldest(&mut self) {
        while self.tracked.len() > TRACKED {
            let Some(oldest) = self
                .tracked
                .iter()
                .min_by_key(|(_, tracked)| tracked.changed_at)
                .map(|(id, _)| id.clone())
            else {
                return;
            };
            self.tracked.remove(&oldest);
        }
    }
}

impl Tracked {
    fn new(session_id: &str, agent_id: &str, row: &Waiting, now: Instant) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            command: short(&row.command_line),
            resource: row.reason,
            held: false,
            want: Want::Waiting,
            told_waiting: false,
            changed_at: now,
        }
    }

    fn notice(&self, process_id: &str, state: &'static str) -> Notice {
        let resource = noun(self.resource);
        let command = &self.command;
        let message = match (state, self.held) {
            ("waiting", false) => format!(
                "`{command}` is paused: the machine is short of {resource}. It continues when {resource} frees."
            ),
            ("waiting", true) => format!(
                "`{command}` is waiting to start: the machine is short of {resource}. It starts when {resource} frees."
            ),
            (_, false) => format!("`{command}` continues."),
            (_, true) => format!("`{command}` starts."),
        };
        Notice {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            process_id: process_id.into(),
            command: self.command.clone(),
            resource,
            state: if state == "waiting" {
                "paused"
            } else {
                "running"
            },
            message,
        }
    }
}

fn noun(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::Memory => "memory",
        WaitReason::Disk => "disk space",
    }
}

/// The command as the agent typed it: its first line, short, with no quote
/// that would break the message.
fn short(command_line: &str) -> String {
    let line = command_line.lines().next().unwrap_or_default().trim();
    let line = line.replace('`', "'");
    if line.chars().count() <= COMMAND_CHARS {
        return line;
    }
    let cut: String = line.chars().take(COMMAND_CHARS).collect();
    format!("{cut}…")
}

fn span(ms: u64) -> String {
    match ms / 1000 {
        seconds @ 0..=59 => format!("{seconds} s"),
        seconds => format!("{} min", seconds / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(command: &str, since_ms: u64) -> Waiting {
        Waiting {
            reason: WaitReason::Memory,
            position: 1,
            need_bytes: 0,
            free_bytes: 0,
            command_line: command.into(),
            cwd: None,
            since_ms,
        }
    }

    fn event(change: WaitChange, state: SessionProcessState) -> WaitEvent {
        WaitEvent {
            keys: Vec::new(),
            process_id: "p1".into(),
            change,
            state,
        }
    }

    fn paused(command: &str) -> WaitEvent {
        event(
            WaitChange::Began,
            SessionProcessState::Paused(row(command, 0)),
        )
    }

    fn resumed() -> WaitEvent {
        event(
            WaitChange::Resumed { waited_ms: 4000 },
            SessionProcessState::Running,
        )
    }

    fn secs(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn a_pause_and_its_end_are_one_message_each_once_they_settle() {
        let (mut notices, t0) = (WaitNotices::default(), Instant::now());
        notices.record("room", "agent", &paused("cargo test"), t0);
        assert!(
            notices.due(secs(t0, 1)).is_empty(),
            "told before it settled"
        );
        let told = notices.due(secs(t0, 4));
        assert_eq!(told.len(), 1);
        assert_eq!(
            told[0].message,
            "`cargo test` is paused: the machine is short of memory. It continues when memory frees."
        );
        assert!(notices.due(secs(t0, 9)).is_empty(), "told twice");

        notices.record("room", "agent", &resumed(), secs(t0, 10));
        let told = notices.due(secs(t0, 14));
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].message, "`cargo test` continues.");
        assert!(notices.due(secs(t0, 30)).is_empty());
    }

    #[test]
    fn a_pause_that_ends_inside_the_settle_time_is_not_told_at_all() {
        let (mut notices, t0) = (WaitNotices::default(), Instant::now());
        notices.record("room", "agent", &paused("cargo test"), t0);
        notices.record("room", "agent", &resumed(), secs(t0, 1));
        assert!(notices.due(secs(t0, 10)).is_empty());
    }

    #[test]
    fn flips_after_a_told_pause_are_told_by_the_state_they_settle_in() {
        let (mut notices, t0) = (WaitNotices::default(), Instant::now());
        notices.record("room", "agent", &paused("cargo test"), t0);
        assert_eq!(notices.due(secs(t0, 4)).len(), 1);
        notices.record("room", "agent", &resumed(), secs(t0, 10));
        notices.record("room", "agent", &paused("cargo test"), secs(t0, 11));
        assert!(
            notices.due(secs(t0, 20)).is_empty(),
            "a resume and a new pause inside the settle time were told"
        );
        notices.record("room", "agent", &resumed(), secs(t0, 30));
        assert_eq!(
            notices.due(secs(t0, 34))[0].message,
            "`cargo test` continues."
        );
    }

    #[test]
    fn an_ended_pause_is_told_at_once_with_how_long_it_lasted() {
        let (mut notices, t0) = (WaitNotices::default(), Instant::now());
        notices.record("room", "agent", &paused("cargo test"), t0);
        let since = crate::session::unix_epoch_ms().saturating_sub(61_000);
        let ended = notices
            .record(
                "room",
                "agent",
                &event(
                    WaitChange::Stopped {
                        reason: "chariox: stopped this command".into(),
                    },
                    SessionProcessState::Paused(row("cargo test", since)),
                ),
                secs(t0, 1),
            )
            .expect("an end is told at once");
        assert_eq!(
            ended.message,
            "`cargo test` was ended after 1 min paused: nothing freed memory. Run it again."
        );
        assert!(
            notices.due(secs(t0, 60)).is_empty(),
            "the pause was told after its end"
        );
    }

    #[test]
    fn a_held_command_says_it_waits_to_start_and_then_that_it_starts() {
        let (mut notices, t0) = (WaitNotices::default(), Instant::now());
        notices.record(
            "room",
            "agent",
            &event(
                WaitChange::Began,
                SessionProcessState::Held(row("make -j8", 0)),
            ),
            t0,
        );
        assert_eq!(
            notices.due(secs(t0, 4))[0].message,
            "`make -j8` is waiting to start: the machine is short of memory. It starts when memory frees."
        );
        notices.record("room", "agent", &resumed(), secs(t0, 5));
        assert_eq!(notices.due(secs(t0, 9))[0].message, "`make -j8` starts.");
    }

    #[test]
    fn a_long_multi_line_command_is_cut_to_its_first_line() {
        let command = format!("echo `date`\n{}", "x".repeat(500));
        assert_eq!(short(&command), "echo 'date'");
        assert!(short(&"y".repeat(500)).chars().count() <= COMMAND_CHARS + 1);
    }
}
