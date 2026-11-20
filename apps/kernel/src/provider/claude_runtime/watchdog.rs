use std::time::{Duration, Instant};

use crate::provider::claude::CLAUDE_START_BUDGET;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClaudeTurnStallAction {
    Wait,
    Restart,
    Fail,
    /// The process did not end its start phase within `CLAUDE_START_BUDGET`.
    FailStart,
}

/// A turn's start state across a hot reload: whether the process ended its
/// start phase and since when the turn waits on it. `Instant`s do not cross
/// the `exec`, so the wait is carried in wall-clock time; the reload itself
/// counts against the start budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ClaudeStartState {
    process_started: bool,
    waiting_since_unix_ms: Option<u64>,
}

#[derive(Debug, Default)]
pub(super) struct ClaudeTurnWatchdog {
    last_activity_at: Option<Instant>,
    /// When the turn began waiting on this process: its start or restart.
    waiting_since: Option<Instant>,
    restart_count: u8,
    saw_runtime_message: bool,
    /// The process ended its start phase by reporting `init`. Until then the
    /// start phase has exactly one bound, `CLAUDE_START_BUDGET`; the stall
    /// timeout arms only after it.
    process_started: bool,
}

impl ClaudeTurnWatchdog {
    pub(super) fn begin(&mut self, now: Instant) {
        self.last_activity_at = Some(now);
        self.waiting_since = Some(now);
        self.restart_count = 0;
        self.saw_runtime_message = false;
    }

    pub(super) fn record_runtime_message(&mut self, now: Instant) {
        if self.last_activity_at.is_some() {
            self.last_activity_at = Some(now);
            self.saw_runtime_message = true;
        }
    }

    pub(super) fn record_process_spawn(&mut self) {
        self.process_started = false;
    }

    pub(super) fn record_process_init(&mut self) {
        self.process_started = true;
    }

    pub(super) fn record_restart(&mut self, now: Instant) {
        self.last_activity_at = Some(now);
        self.waiting_since = Some(now);
        self.restart_count = self.restart_count.saturating_add(1);
        self.saw_runtime_message = false;
    }

    pub(super) fn start_state(&self, now: Instant, now_unix_ms: u64) -> ClaudeStartState {
        ClaudeStartState {
            process_started: self.process_started,
            waiting_since_unix_ms: self.waiting_since.map(|since| {
                let waited = u64::try_from(now.saturating_duration_since(since).as_millis())
                    .unwrap_or(u64::MAX);
                now_unix_ms.saturating_sub(waited)
            }),
        }
    }

    /// Continues a handed-over turn with the start state it had.
    pub(super) fn adopt(&mut self, start: ClaudeStartState, now: Instant, now_unix_ms: u64) {
        self.begin(now);
        self.process_started = start.process_started;
        self.waiting_since = start.waiting_since_unix_ms.map(|since| {
            now.checked_sub(Duration::from_millis(now_unix_ms.saturating_sub(since)))
                .unwrap_or(now)
        });
    }

    pub(super) fn settle(&mut self) {
        self.last_activity_at = None;
        self.waiting_since = None;
        self.restart_count = 0;
        self.saw_runtime_message = false;
    }

    pub(super) fn action(&self, now: Instant, stall_timeout: Duration) -> ClaudeTurnStallAction {
        let (Some(last_activity_at), Some(waiting_since)) =
            (self.last_activity_at, self.waiting_since)
        else {
            return ClaudeTurnStallAction::Wait;
        };
        if !self.process_started {
            return if now.saturating_duration_since(waiting_since) < CLAUDE_START_BUDGET {
                ClaudeTurnStallAction::Wait
            } else {
                ClaudeTurnStallAction::FailStart
            };
        }
        if now.saturating_duration_since(last_activity_at) < stall_timeout {
            return ClaudeTurnStallAction::Wait;
        }
        if self.restart_count == 0 && !self.saw_runtime_message {
            ClaudeTurnStallAction::Restart
        } else {
            ClaudeTurnStallAction::Fail
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STALL: Duration = Duration::from_secs(60);

    fn started_process(now: Instant) -> ClaudeTurnWatchdog {
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.record_process_init();
        watchdog.begin(now);
        watchdog
    }

    #[test]
    fn restarts_once_when_claude_never_acknowledges_the_turn() {
        let started = Instant::now();
        let mut watchdog = started_process(started);

        assert_eq!(
            watchdog.action(started + STALL, STALL),
            ClaudeTurnStallAction::Restart
        );
        watchdog.record_restart(started + STALL);
        assert_eq!(
            watchdog.action(started + 2 * STALL, STALL),
            ClaudeTurnStallAction::Fail
        );
    }

    #[test]
    fn a_new_process_keeps_its_turn_for_the_whole_start_budget() {
        let started = Instant::now();
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.begin(started);

        // Claude stays silent while it connects its MCP servers, up to
        // `MCP_TIMEOUT`: that is its start phase, not a stall.
        assert_eq!(
            watchdog.action(started + Duration::from_secs(61), STALL),
            ClaudeTurnStallAction::Wait
        );
        watchdog.record_runtime_message(started + Duration::from_secs(75));
        watchdog.record_process_init();
        assert_eq!(
            watchdog.action(started + Duration::from_secs(75) + STALL, STALL),
            ClaudeTurnStallAction::Fail
        );
    }

    #[test]
    fn a_start_phase_past_its_budget_fails_without_a_restart() {
        let started = Instant::now();
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.begin(started);
        // Output before `init` does not extend the start phase.
        watchdog.record_runtime_message(started + Duration::from_secs(100));

        assert_eq!(
            watchdog.action(started + CLAUDE_START_BUDGET, STALL),
            ClaudeTurnStallAction::FailStart
        );
    }

    #[test]
    fn a_restarted_process_has_a_start_phase_of_its_own() {
        let started = Instant::now();
        let mut watchdog = started_process(started);
        watchdog.record_process_spawn();
        watchdog.record_restart(started + STALL);

        assert_eq!(
            watchdog.action(started + 2 * STALL, STALL),
            ClaudeTurnStallAction::Wait
        );
        assert_eq!(
            watchdog.action(started + STALL + CLAUDE_START_BUDGET, STALL),
            ClaudeTurnStallAction::FailStart
        );
    }

    #[test]
    fn never_replays_a_turn_after_any_runtime_message() {
        let started = Instant::now();
        let mut watchdog = started_process(started);
        watchdog.record_runtime_message(started + Duration::from_secs(10));

        assert_eq!(
            watchdog.action(started + Duration::from_secs(70), STALL),
            ClaudeTurnStallAction::Fail
        );
    }

    #[test]
    fn settled_watchdogs_stay_idle() {
        let started = Instant::now();
        let mut watchdog = started_process(started);
        watchdog.settle();

        assert_eq!(
            watchdog.action(started + 2 * STALL, STALL),
            ClaudeTurnStallAction::Wait
        );
    }
}
