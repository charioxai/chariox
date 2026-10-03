//! Poll an idle App bridge at its existing cadence; a call buys a short
//! active window. No page can request an indefinite faster polling lease.
use std::time::Duration;
use tokio::time::{Instant, Interval, MissedTickBehavior};

const IDLE_INTERVAL: Duration = Duration::from_millis(250);
const ACTIVE_INTERVAL: Duration = Duration::from_millis(25);
const ACTIVE_WINDOW: Duration = Duration::from_secs(1);

pub(super) struct AppViewPoll {
    ticks: Interval,
    active_until: Option<Instant>,
    active: bool,
}

impl AppViewPoll {
    pub(super) fn new() -> Self {
        let mut ticks = tokio::time::interval(IDLE_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        Self {
            ticks,
            active_until: None,
            active: false,
        }
    }

    pub(super) async fn tick(&mut self) {
        self.ticks.tick().await;
    }

    pub(super) fn observed_calls(&mut self, had_calls: bool) {
        let now = Instant::now();
        if had_calls {
            self.active_until = Some(now + ACTIVE_WINDOW);
        }
        let active = self.active_until.is_some_and(|until| now < until);
        if active != self.active {
            self.active = active;
            self.reset(if active {
                ACTIVE_INTERVAL
            } else {
                IDLE_INTERVAL
            });
        }
    }

    pub(super) fn failed(&mut self) {
        self.active_until = None;
        self.active = false;
        // Twenty failed retries retain the old roughly five-second window.
        self.reset(IDLE_INTERVAL);
    }

    fn reset(&mut self, interval: Duration) {
        self.ticks = tokio::time::interval_at(Instant::now() + interval, interval);
        self.ticks
            .set_missed_tick_behavior(MissedTickBehavior::Skip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn first_poll_is_immediate_then_idle_polls_keep_the_old_cadence() {
        let began = Instant::now();
        let mut polls = AppViewPoll::new();
        polls.tick().await;
        assert_eq!(Instant::now(), began);
        for n in 1..=4 {
            polls.observed_calls(false);
            polls.tick().await;
            assert_eq!(Instant::now() - began, IDLE_INTERVAL * n);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn calls_buy_a_bounded_fast_window_then_return_to_idle() {
        let mut polls = AppViewPoll::new();
        polls.tick().await;
        let began = Instant::now();
        polls.observed_calls(true);
        for n in 1..=40 {
            polls.tick().await;
            assert_eq!(Instant::now() - began, ACTIVE_INTERVAL * n);
            polls.observed_calls(false);
        }
        polls.tick().await;
        assert_eq!(Instant::now() - began, ACTIVE_WINDOW + IDLE_INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_rpc_skips_missed_active_ticks_without_a_catch_up_burst() {
        let mut polls = AppViewPoll::new();
        polls.tick().await;
        polls.observed_calls(true);
        let began = Instant::now();
        tokio::time::advance(Duration::from_millis(251)).await;
        polls.tick().await;
        assert_eq!(Instant::now() - began, Duration::from_millis(251));
        polls.tick().await;
        assert_eq!(Instant::now() - began, Duration::from_millis(275));
    }

    #[tokio::test(start_paused = true)]
    async fn failures_end_fast_polling_and_keep_twenty_retries_at_five_seconds() {
        let mut polls = AppViewPoll::new();
        polls.tick().await;
        polls.observed_calls(true);
        let began = Instant::now();
        for n in 1..=20 {
            polls.failed();
            polls.tick().await;
            assert_eq!(Instant::now() - began, IDLE_INTERVAL * n);
        }
    }
}
