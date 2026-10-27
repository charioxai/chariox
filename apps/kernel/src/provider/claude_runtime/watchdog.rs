use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClaudeTurnStallAction {
    Wait,
    Restart,
    Fail,
}

#[derive(Debug, Default)]
pub(super) struct ClaudeTurnWatchdog {
    last_activity_at: Option<Instant>,
    restart_count: u8,
    saw_runtime_message: bool,
}

impl ClaudeTurnWatchdog {
    pub(super) fn begin(&mut self, now: Instant) {
        self.last_activity_at = Some(now);
        self.restart_count = 0;
        self.saw_runtime_message = false;
    }

    pub(super) fn record_runtime_message(&mut self, now: Instant) {
        if self.last_activity_at.is_some() {
            self.last_activity_at = Some(now);
            self.saw_runtime_message = true;
        }
    }

    pub(super) fn record_restart(&mut self, now: Instant) {
        self.last_activity_at = Some(now);
        self.restart_count = self.restart_count.saturating_add(1);
        self.saw_runtime_message = false;
    }

    pub(super) fn settle(&mut self) {
        self.last_activity_at = None;
        self.restart_count = 0;
        self.saw_runtime_message = false;
    }

    pub(super) fn action(&self, now: Instant, stall_timeout: Duration) -> ClaudeTurnStallAction {
        let Some(last_activity_at) = self.last_activity_at else {
            return ClaudeTurnStallAction::Wait;
        };
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

    #[test]
    fn restarts_once_when_claude_never_acknowledges_the_turn() {
        let started = Instant::now();
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.begin(started);

        assert_eq!(
            watchdog.action(started + Duration::from_secs(60), Duration::from_secs(60)),
            ClaudeTurnStallAction::Restart
        );
        watchdog.record_restart(started + Duration::from_secs(60));
        assert_eq!(
            watchdog.action(started + Duration::from_secs(120), Duration::from_secs(60)),
            ClaudeTurnStallAction::Fail
        );
    }

    #[test]
    fn never_replays_a_turn_after_any_runtime_message() {
        let started = Instant::now();
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.begin(started);
        watchdog.record_runtime_message(started + Duration::from_secs(10));

        assert_eq!(
            watchdog.action(started + Duration::from_secs(70), Duration::from_secs(60)),
            ClaudeTurnStallAction::Fail
        );
    }

    #[test]
    fn settled_watchdogs_stay_idle() {
        let started = Instant::now();
        let mut watchdog = ClaudeTurnWatchdog::default();
        watchdog.begin(started);
        watchdog.settle();

        assert_eq!(
            watchdog.action(started + Duration::from_secs(120), Duration::from_secs(60)),
            ClaudeTurnStallAction::Wait
        );
    }
}
