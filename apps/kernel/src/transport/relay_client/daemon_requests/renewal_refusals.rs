//! MP-08/MP-11: successful renewals stay quiet; refusals expose only safe reasons.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chariox_relay::protocol::RelayError;

const WARNING_INTERVAL: Duration = Duration::from_secs(10);
static LAST_WARNING: Mutex<Option<Instant>> = Mutex::new(None);

#[cfg(test)]
pub(in crate::transport::relay_client) fn reset_for_test() {
    *LAST_WARNING.lock().unwrap() = None;
}

pub(super) fn log(error: &RelayError) {
    log_at(&LAST_WARNING, error, Instant::now());
}

fn log_at(last_warning: &Mutex<Option<Instant>>, error: &RelayError, now: Instant) {
    {
        let mut last = last_warning.lock().expect("renewal refusal log poisoned");
        if last.is_some_and(|previous| now.duration_since(previous) < WARNING_INTERVAL) {
            return;
        }
        *last = Some(now);
    }
    // Never include the grant, identity, sender key or encrypted request.
    crate::logging::warn_with_fields(
        "daemon.relay_client",
        "local browser lease renewal refused",
        serde_json::json!({
            "request_kind": "local_browser_renew",
            "reason_code": error.code,
            "reason": error.message,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp11_renewal_refusal_warnings_are_rate_limited() {
        let capture = crate::logging::capture::start();
        let error =
            super::super::relay_error("mp11_test_refusal", "synchronize the system clock", false);
        let last = Mutex::new(None);
        let now = Instant::now();
        log_at(&last, &error, now);
        log_at(&last, &error, now);
        log_at(
            &last,
            &error,
            now + WARNING_INTERVAL - Duration::from_millis(1),
        );
        log_at(&last, &error, now + WARNING_INTERVAL);
        let records: Vec<_> = capture
            .records()
            .into_iter()
            .filter(|record| record.contains("mp11_test_refusal"))
            .collect();
        assert_eq!(
            records.len(),
            2,
            "first refusal and interval expiry must log"
        );
        for record in records {
            assert!(record.contains("local browser lease renewal refused"));
            assert!(record.contains("local_browser_renew"));
            assert!(record.contains("synchronize the system clock"));
            assert!(!record.contains("grant"));
        }
    }
}
