//! Registration is incomplete until the relay confirms it can route to this kernel.

use super::*;
use crate::local::RelayRegistrationRefusal;

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(10);
/// A refused kernel retries soon, then every 30 s: the relay it dials can be replaced
/// at any moment, and the kernel must register within a minute of that however long
/// it was refused. That is still no faster than twice the once-a-minute retry the
/// relay's own refusal record expects (docs/PROTOCOL.md). Miguel decision 11-23.
const REGISTRATION_RETRY_SECONDS: [u64; 2] = [15, 30];

pub(super) async fn await_registration(
    inbound: &mut mpsc::Receiver<Message>,
    reader_end: &mut oneshot::Receiver<socket_thread::ReaderEnd>,
    writer_done: &mut oneshot::Receiver<()>,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<Result<(), String>> {
    tokio::select! {
        biased;
        _ = shutdown.changed() => None,
        Some(frame) = inbound.recv() => Some(match frame {
            Message::Text(text)
                if matches!(serde_json::from_str(&text), Ok(RelayEnvelope::DaemonRegistered)) =>
            {
                Ok(())
            }
            _ => Err("relay sent traffic before acknowledging registration".into()),
        }),
        end = reader_end => Some(Err(match end {
            Ok(socket_thread::ReaderEnd::RelayClose(reason)) => reason,
            Ok(socket_thread::ReaderEnd::SocketClose(Some(frame))) if !frame.reason.is_empty() =>
                frame.reason.to_string(),
            _ => "relay closed before acknowledging registration".into(),
        })),
        _ = writer_done => Some(Err("relay writer ended before registration".into())),
        _ = sleep(REGISTRATION_TIMEOUT) =>
            Some(Err("relay did not acknowledge registration".into())),
    }
}

fn registration_retry_delay(attempt: usize, daemon_id: &str) -> Duration {
    let step = attempt.min(REGISTRATION_RETRY_SECONDS.len() - 1);
    Duration::from_secs(REGISTRATION_RETRY_SECONDS[step])
        + Duration::from_millis(stable_jitter_ms(
            daemon_id,
            RELAY_RECONNECT_JITTER_SPREAD_MS,
        ))
}

#[derive(Default)]
pub(super) struct RefusalBackoff {
    current: Option<(String, RelayRegistrationRefusal)>,
    attempt: usize,
    owner_notified: bool,
}

impl RefusalBackoff {
    pub(super) async fn record(
        &mut self,
        router: &CommandRouter,
        state: &RwLock<RelayClientState>,
        relay_url: &str,
        reason: String,
        token_refused: bool,
    ) -> (Duration, bool) {
        let previous = self
            .current
            .as_ref()
            .filter(|(url, refusal)| url == relay_url && refusal.reason == reason);
        let now = crate::session::unix_epoch_ms();
        let since_ms = previous.map_or(now, |(_, refusal)| refusal.since_ms);
        let new_refusal = previous.is_none();
        if new_refusal {
            self.attempt = 0;
            self.owner_notified = false;
        }
        let next_step = if token_refused {
            "renew the relay connection credentials"
        } else if reason.starts_with("upgrade required:")
            || reason.contains("missing registration proof")
        {
            "update this kernel"
        } else if reason == "relay did not acknowledge registration"
            || reason.contains("offered no registration challenge")
        {
            "update the relay and check its health"
        } else {
            "check the relay and kernel configuration"
        };
        let delay = registration_retry_delay(self.attempt, &router.relay_daemon_id());
        self.attempt = (self.attempt + 1).min(REGISTRATION_RETRY_SECONDS.len() - 1);
        let refusal = RelayRegistrationRefusal {
            reason,
            next_step: next_step.into(),
            since_ms,
            next_retry_at_ms: (!token_refused)
                .then(|| now.saturating_add(delay.as_millis() as u64)),
        };
        let message = refusal.owner_message();
        state
            .write()
            .await
            .set_registration_refusal(relay_url, refusal.clone());
        if new_refusal {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                message.clone(),
                serde_json::json!({"relay_url": relay_url}),
            );
        }
        if !self.owner_notified {
            self.owner_notified = router
                .runtime_state()
                .announce_relay_registration_refusal(&message);
        }
        if let Some(next_retry_at_ms) = refusal.next_retry_at_ms {
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay registration retry scheduled",
                serde_json::json!({"relay_url": relay_url, "next_retry_at_ms": next_retry_at_ms}),
            );
            record_relay_reconnect(router, relay_url, &refusal.reason, delay);
        }
        self.current = Some((relay_url.into(), refusal));
        (delay, new_refusal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLACED_RELAY_REGISTERS_WITHIN: Duration = Duration::from_secs(60);
    const ACK_WAIT: Duration = REGISTRATION_TIMEOUT;

    #[test]
    fn a_kernel_refused_for_a_long_time_still_retries_within_a_minute_of_a_relay_replacement() {
        for attempt in [0, 1, 2, 3, 10, 1_000] {
            let wait = registration_retry_delay(attempt, "kernel-a");
            assert!(
                wait < REPLACED_RELAY_REGISTERS_WITHIN,
                "refusal {attempt} waits {wait:?} before the next registration"
            );
        }
    }

    #[test]
    fn a_relay_that_keeps_refusing_is_dialed_no_faster_than_the_first_step() {
        let first = Duration::from_secs(REGISTRATION_RETRY_SECONDS[0]);
        let attempts_in_ten_minutes = (0..100)
            .scan(Duration::ZERO, |elapsed, attempt| {
                *elapsed += registration_retry_delay(attempt, "kernel-a") + ACK_WAIT;
                (*elapsed <= Duration::from_secs(600)).then_some(())
            })
            .count();
        assert!(registration_retry_delay(0, "kernel-a") >= first);
        assert!(
            attempts_in_ten_minutes <= 21,
            "{attempts_in_ten_minutes} attempts in ten minutes"
        );
    }
}
