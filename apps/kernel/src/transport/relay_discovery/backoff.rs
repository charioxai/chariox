//! Spaces out relay metadata calls after the relay could not be reached, so a
//! poller does not open a new TCP connection to a struggling relay on every tick.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::time::Instant;

const FIRST_DELAY: Duration = Duration::from_secs(1);
const CEILING: Duration = Duration::from_secs(30);
/// Bounds the table when a kernel is pointed at many relay URLs over its life.
const MAX_RELAYS: usize = 64;

struct Entry {
    failures: u32,
    blocked_until: Instant,
    failure: Failure,
}

/// The dial failure that started a relay's delay.
#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) operation: &'static str,
    pub(super) message: String,
}

/// A poll refused while a relay's delay runs: the time left, and the failure a
/// refused poll reports again so its caller reads it as it read the dial.
#[derive(Debug)]
pub(super) struct Refused {
    pub(super) left: Duration,
    pub(super) failure: Failure,
}

/// Failure count and refusal window per relay URL. Callers never hold the lock
/// across an await.
#[derive(Default)]
pub(super) struct ConnectBackoff {
    relays: Mutex<HashMap<String, Entry>>,
}

impl ConnectBackoff {
    pub(super) fn shared() -> &'static Self {
        static SHARED: OnceLock<ConnectBackoff> = OnceLock::new();
        SHARED.get_or_init(Self::default)
    }

    pub(super) fn admit(&self, relay: &str, now: Instant) -> Result<(), Refused> {
        let relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        match relays.get(relay) {
            Some(entry) if entry.blocked_until > now => Err(Refused {
                left: entry.blocked_until - now,
                failure: entry.failure.clone(),
            }),
            _ => Ok(()),
        }
    }

    /// `jitter` in `0.5..=1.0` scales the delay so pollers do not retry in step.
    pub(super) fn record_failure(&self, relay: &str, now: Instant, jitter: f64, failure: Failure) {
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        if relays.len() >= MAX_RELAYS && !relays.contains_key(relay) {
            relays.retain(|_, entry| entry.blocked_until > now);
        }
        if relays.len() >= MAX_RELAYS && !relays.contains_key(relay) {
            return;
        }
        let failures = relays
            .get(relay)
            .map_or(1, |entry| entry.failures.saturating_add(1));
        let doubling = 1u32 << (failures - 1).min(5);
        let delay = (FIRST_DELAY * doubling).min(CEILING);
        relays.insert(
            relay.to_string(),
            Entry {
                failures,
                blocked_until: now + delay.mul_f64(jitter.clamp(0.5, 1.0)),
                failure,
            },
        );
    }

    pub(super) fn record_success(&self, relay: &str) {
        self.relays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(relay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused() -> Failure {
        Failure {
            operation: "connect relay metadata socket",
            message: "Connection refused (os error 111)".into(),
        }
    }

    #[test]
    fn a_reachable_relay_is_always_admitted() {
        let backoff = ConnectBackoff::default();
        assert!(backoff.admit("ws://relay", Instant::now()).is_ok());
    }

    #[test]
    fn a_failed_relay_is_refused_until_the_delay_has_passed_and_success_clears_it() {
        let backoff = ConnectBackoff::default();
        let start = Instant::now();
        backoff.record_failure("ws://relay", start, 1.0, refused());
        let left = backoff
            .admit("ws://relay", start)
            .expect_err("a failed relay is refused")
            .left;
        assert_eq!(left, FIRST_DELAY);
        assert!(backoff.admit("ws://other", start).is_ok());
        assert!(backoff.admit("ws://relay", start + FIRST_DELAY).is_ok());
        backoff.record_failure("ws://relay", start, 1.0, refused());
        backoff.record_success("ws://relay");
        assert!(backoff.admit("ws://relay", start).is_ok());
    }

    #[test]
    fn ten_minutes_of_one_second_polls_dial_a_dead_relay_a_bounded_number_of_times() {
        let backoff = ConnectBackoff::default();
        let start = Instant::now();
        let mut dials = 0;
        let mut longest = Duration::ZERO;
        for second in 0..600 {
            let now = start + Duration::from_secs(second);
            if backoff.admit("ws://relay", now).is_ok() {
                dials += 1;
                backoff.record_failure("ws://relay", now, 1.0, refused());
                longest = longest.max(backoff.admit("ws://relay", now).unwrap_err().left);
            }
        }
        assert!(dials <= 28, "dialed {dials} times in ten minutes");
        assert_eq!(longest, CEILING, "the delay stops growing at the ceiling");
    }

    #[test]
    fn jitter_shortens_the_delay_but_never_below_half() {
        let backoff = ConnectBackoff::default();
        let start = Instant::now();
        backoff.record_failure("ws://a", start, 0.0, refused());
        backoff.record_failure("ws://b", start, 7.0, refused());
        assert_eq!(
            backoff.admit("ws://a", start).unwrap_err().left,
            FIRST_DELAY / 2
        );
        assert_eq!(
            backoff.admit("ws://b", start).unwrap_err().left,
            FIRST_DELAY
        );
    }

    #[test]
    fn the_table_stays_bounded_when_every_relay_is_new() {
        let backoff = ConnectBackoff::default();
        let start = Instant::now();
        for index in 0..(MAX_RELAYS * 3) {
            backoff.record_failure(&format!("ws://relay-{index}"), start, 1.0, refused());
        }
        let len = backoff.relays.lock().unwrap().len();
        assert!(len <= MAX_RELAYS, "{len} relays tracked");
    }
}
