use std::time::Duration;

use super::{
    wait_for_poll_or_shutdown, PollWaitOutcome, QuiescencePollSchedule, QuiescencePollWarningState,
    MAX_FAILURE_POLL_INTERVAL, POLL_INTERVAL,
};

#[test]
fn successful_poll_keeps_the_two_second_cadence() {
    let schedule = QuiescencePollSchedule::default();

    assert_eq!(schedule.next_poll_delay(), POLL_INTERVAL);
    assert_eq!(schedule.next_poll_delay(), Duration::from_secs(2));
}

#[test]
fn consecutive_failures_back_off_to_a_bounded_delay() {
    let mut schedule = QuiescencePollSchedule::default();
    let expected_delays = [2, 4, 8, 16, 30, 30];

    for (index, expected_seconds) in expected_delays.into_iter().enumerate() {
        schedule.record_failure();

        assert_eq!(schedule.consecutive_failures, (index + 1) as u32);
        assert_eq!(
            schedule.next_poll_delay(),
            Duration::from_secs(expected_seconds),
        );
    }
    assert_eq!(schedule.next_poll_delay(), MAX_FAILURE_POLL_INTERVAL);
}

#[test]
fn successful_poll_resets_the_consecutive_failure_backoff() {
    let mut schedule = QuiescencePollSchedule::default();
    schedule.record_failure();
    schedule.record_failure();
    schedule.record_failure();
    assert_eq!(schedule.next_poll_delay(), Duration::from_secs(8));

    schedule.record_success();

    assert_eq!(schedule.consecutive_failures, 0);
    assert_eq!(schedule.next_poll_delay(), POLL_INTERVAL);
    schedule.record_failure();
    assert_eq!(schedule.next_poll_delay(), POLL_INTERVAL);
}

#[test]
fn retry_warnings_are_summarized_and_reset_after_success() {
    let mut warnings = QuiescencePollWarningState::default();

    assert!(warnings.should_warn());
    for _ in 0..4 {
        assert!(!warnings.should_warn());
    }
    assert!(warnings.should_warn());

    warnings.reset();
    assert!(warnings.should_warn());
}

#[tokio::test]
async fn shutdown_watch_interrupts_a_failure_backoff_wait() {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    shutdown_tx.send(true).unwrap();

    let outcome = wait_for_poll_or_shutdown(&mut shutdown_rx, MAX_FAILURE_POLL_INTERVAL).await;

    assert_eq!(outcome, PollWaitOutcome::Shutdown);
}
