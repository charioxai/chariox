//! MP-08/MP-10: protocol 475 pushed display. One kernel-resident pump per
//! relay display subscription takes frames through the ordinary admitted
//! display path (terminal admission, Vault barrier, host policy, scrub) and
//! pushes them as relay events. The relay RTT leaves the frame loop; the
//! client's acknowledgements gate it, after Selkies' frame ACK backpressure
//! (selkies websockets_mode.py `_run_frame_backpressure_logic`, MPL-2.0).
use super::*;
use crate::runtime::state::{KernelBrowserDisplayRequest, KernelBrowserPushCredit};
use std::collections::VecDeque;

/// Unacknowledged frames older than the RTT floor plus this allowance close
/// the gate: queueing beyond it only adds input-to-visible latency.
const GATE_ALLOWANCE: Duration = Duration::from_millis(100);
/// Selkies STALLED_CLIENT_TIMEOUT_SECONDS / STALLED_CLIENT_REPROBE_SECONDS.
const STALL: Duration = Duration::from_secs(4);
const REPROBE: Duration = Duration::from_secs(2);
const RTT_WINDOW: Duration = Duration::from_secs(10);
/// Two gate closures within this window lower the motion bitrate.
const CONGESTION_WINDOW: Duration = Duration::from_secs(2);
/// Consecutive host failures before the pump stops and the client resubscribes.
const FAILURE_LIMIT: u32 = 5;
/// Selkies VIDEO_RELAY_SYNC_FLOOR_SECONDS: at most one key request per second.
const SYNC_FLOOR: Duration = Duration::from_secs(1);

#[derive(Default)]
struct PumpState {
    generation: u64,
    started: bool,
    stopped: bool,
    failed: bool,
    acked: u64,
    reset: bool,
    sent: VecDeque<(u64, Instant)>,
    rtt: VecDeque<(Instant, Duration)>,
    gated_at: Option<Instant>,
    closes: VecDeque<Instant>,
    synced_at: Option<Instant>,
}

#[derive(Debug, PartialEq)]
enum Gate {
    Closed,
    Open(KernelBrowserPushCredit),
}

impl PumpState {
    fn rtt_floor(&self) -> Duration {
        self.rtt
            .iter()
            .map(|(_, rtt)| *rtt)
            .min()
            .unwrap_or_default()
    }
    fn admit(&mut self, now: Instant) -> Gate {
        if let Some(&(_, oldest)) = self.sent.front() {
            let age = now.saturating_duration_since(oldest);
            if age > STALL {
                // A stalled gate is sent nothing; reopen on an independent
                // frame after the reprobe interval (Selkies semantics).
                let gated = *self.gated_at.get_or_insert(now);
                if now.saturating_duration_since(gated) < REPROBE {
                    return Gate::Closed;
                }
                self.sent.clear();
                self.reset = true;
            } else if age > self.rtt_floor() + GATE_ALLOWANCE {
                if self.gated_at.is_none() {
                    self.gated_at = Some(now);
                    self.closes.push_back(now);
                }
                return Gate::Closed;
            }
        }
        self.gated_at = None;
        while self
            .closes
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > CONGESTION_WINDOW)
        {
            self.closes.pop_front();
        }
        // A pending reset waits for the sync floor; it stays requested.
        let reset = self.reset
            && self
                .synced_at
                .is_none_or(|at| now.saturating_duration_since(at) >= SYNC_FLOOR);
        if reset {
            self.reset = false;
            self.synced_at = Some(now);
        }
        Gate::Open(KernelBrowserPushCredit {
            reset,
            congested: self.closes.len() >= 2,
        })
    }
    fn acknowledge(&mut self, sequence: u64, lost: bool, now: Instant) {
        self.started = true;
        self.reset |= lost;
        self.acked = self.acked.max(sequence);
        while let Some(&(sent, at)) = self.sent.front() {
            if sent > sequence {
                break;
            }
            self.sent.pop_front();
            if sent == sequence {
                self.rtt.push_back((now, now.saturating_duration_since(at)));
            }
        }
        while self
            .rtt
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > RTT_WINDOW)
        {
            self.rtt.pop_front();
        }
    }
}

pub(super) struct DisplayPump {
    state: std::sync::Mutex<PumpState>,
    wake: tokio::sync::Notify,
}

impl DisplayPump {
    /// A new pump (also a re-subscription after a dropped relay socket, whose
    /// in-flight frames were lost) starts with an independent frame.
    pub(super) fn new(generation: u64) -> Arc<Self> {
        Arc::new(Self {
            state: std::sync::Mutex::new(PumpState {
                generation,
                reset: true,
                ..Default::default()
            }),
            wake: tokio::sync::Notify::new(),
        })
    }
    fn with<T>(&self, f: impl FnOnce(&mut PumpState) -> T) -> T {
        f(&mut self.state.lock().unwrap_or_else(|error| error.into_inner()))
    }
    /// Records the newest presented sequence. `None` refuses a foreign generation.
    pub(super) fn acknowledge(
        &self,
        generation: u64,
        sequence: u64,
        lost: bool,
    ) -> Option<&'static str> {
        let status = self.with(|state| {
            if state.generation != generation {
                return None;
            }
            state.acknowledge(sequence, lost, Instant::now());
            Some(if state.failed {
                "failed"
            } else if state.stopped {
                "stopped"
            } else {
                "running"
            })
        });
        self.wake.notify_one();
        status
    }
    pub(super) fn stop(&self) {
        self.with(|state| state.stopped = true);
        self.wake.notify_one();
    }
}

pub(super) struct PumpRoute {
    pub(super) router: Arc<CommandRouter>,
    pub(super) outgoing_tx: RelayOutgoingSender,
    pub(super) tasks: super::subscriptions::RelaySubscriptionTasks,
    pub(super) key: String,
    pub(super) caller: crate::runtime::command::KernelCommand,
    pub(super) display_id: String,
    pub(super) relay_id: String,
    pub(super) public_key: String,
}

async fn lease_alive(route: &PumpRoute) -> bool {
    route
        .tasks
        .lock()
        .await
        .get(&route.key)
        .is_some_and(|task| !task.handle.is_finished())
}

/// Dormant until the client's first acknowledgement; credit-mode clients never ack.
pub(super) async fn run(route: PumpRoute, pump: Arc<DisplayPump>) {
    let generation = pump.with(|state| state.generation);
    loop {
        if pump.with(|state| state.stopped) || !lease_alive(&route).await {
            return;
        }
        if pump.with(|state| state.started) {
            break;
        }
        let _ = timeout(Duration::from_secs(1), pump.wake.notified()).await;
    }
    let daemon_private_key = route.router.relay_private_key();
    let mut last_sent = pump.with(|state| state.acked);
    let (mut idle, mut failures) = (Duration::ZERO, 0u32);
    loop {
        if pump.with(|state| state.stopped) || !lease_alive(&route).await {
            return;
        }
        let credit = match pump.with(|state| state.admit(Instant::now())) {
            Gate::Closed => {
                let _ = timeout(Duration::from_millis(20), pump.wake.notified()).await;
                continue;
            }
            Gate::Open(credit) => credit,
        };
        let started = Instant::now();
        let result = route
            .router
            .runtime_state()
            .kernel_browser_display_request(
                &route.caller,
                KernelBrowserDisplayRequest::EncodedCapture {
                    subscription_id: route.display_id.clone(),
                    generation,
                    after_sequence: last_sent,
                    push: Some(credit),
                },
            )
            .await;
        let mut response = match result {
            Ok(result) => {
                failures = 0;
                serde_json::json!({"KernelBrowser":{"result":result}})
            }
            // A navigation between capture and commit refuses one credit as
            // a stale document; the next credit binds the new document.
            Err(DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::StaleReference,
            }) => {
                sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(error) => {
                // A gone subscription (stale generation, revoked grant) is
                // final; other host failures retry on an independent frame.
                failures += 1;
                if matches!(error, DaemonError::UserDomainRefused { .. })
                    || failures >= FAILURE_LIMIT
                {
                    pump.with(|state| state.failed = true);
                    return;
                }
                pump.with(|state| state.reset = true);
                sleep(Duration::from_millis(250) * failures).await;
                continue;
            }
        };
        let Some((_, sequence, event)) =
            crate::transport::kernel_browser_display::take_display_event(&mut response)
        else {
            // The host parks each credit on producer readiness; back off only
            // sources (CDP fallback) that answer empty credits immediately.
            if started.elapsed() < Duration::from_millis(5) {
                idle = (idle * 2).clamp(Duration::from_millis(8), Duration::from_millis(100));
                let _ = timeout(idle, pump.wake.notified()).await;
            }
            continue;
        };
        idle = Duration::ZERO;
        let at = std::time::Instant::now();
        let Some(encrypted_event) =
            crate::transport::kernel_browser_display::encode_display_event(event)
                .ok()
                .and_then(|bytes| {
                    relay_crypto::encrypt_payload_for_peer(
                        &daemon_private_key,
                        &route.public_key,
                        &bytes,
                    )
                    .ok()
                })
        else {
            pump.with(|state| state.failed = true);
            return;
        };
        crate::transport::kernel_browser_display::timing("event_serialize_encrypt", at);
        pump.with(|state| state.sent.push_back((sequence, Instant::now())));
        last_sent = sequence;
        if route
            .outgoing_tx
            .send_display_event(RelayEnvelope::DaemonEvent {
                subscription_id: route.relay_id.clone(),
                event_id: sequence,
                encrypted_event,
            })
            .await
            .is_err()
        {
            return;
        }
        crate::transport::kernel_browser_display::timing("event_queue_credit", at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp10_ack_gate_closes_on_lag_and_lifts_after_ack() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(0, false, start);
        assert!(matches!(state.admit(start), Gate::Open(_)));
        for sequence in 1..=3 {
            state.sent.push_back((sequence, start));
        }
        // Outstanding frames within the allowance keep streaming.
        assert!(matches!(
            state.admit(start + Duration::from_millis(50)),
            Gate::Open(_)
        ));
        // A client lagging past RTT floor + allowance stops new frames.
        assert_eq!(
            state.admit(start + Duration::from_millis(150)),
            Gate::Closed
        );
        state.acknowledge(3, false, start + Duration::from_millis(160));
        assert_eq!(state.rtt_floor(), Duration::from_millis(160));
        let Gate::Open(credit) = state.admit(start + Duration::from_millis(161)) else {
            panic!("acknowledged client must reopen the gate");
        };
        assert!(!credit.reset && !credit.congested);
    }

    #[test]
    fn mp10_lost_frames_and_stalls_request_an_independent_frame() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(4, true, start);
        let Gate::Open(credit) = state.admit(start) else {
            panic!()
        };
        assert!(credit.reset, "lost frame must retire the reference chain");
        let Gate::Open(credit) = state.admit(start) else {
            panic!()
        };
        assert!(!credit.reset, "reset is consumed once");
        state.acknowledge(4, true, start);
        let Gate::Open(credit) = state.admit(start + Duration::from_millis(500)) else {
            panic!()
        };
        assert!(
            !credit.reset && state.reset,
            "a second loss waits for the sync floor"
        );
        let Gate::Open(credit) = state.admit(start + SYNC_FLOOR) else {
            panic!()
        };
        assert!(credit.reset);
        state.sent.push_back((5, start + SYNC_FLOOR));
        let start = start + SYNC_FLOOR;
        let stalled = start + STALL + Duration::from_millis(1);
        assert_eq!(state.admit(stalled), Gate::Closed);
        assert_eq!(state.admit(stalled + Duration::from_secs(1)), Gate::Closed);
        let Gate::Open(credit) = state.admit(stalled + REPROBE) else {
            panic!("stalled gate reprobes")
        };
        assert!(credit.reset && state.sent.is_empty());
    }

    #[test]
    fn mp10_repeated_gate_closure_reports_congestion() {
        let start = Instant::now();
        let mut state = PumpState::default();
        // A 5 ms RTT floor: 150 ms of lag exceeds floor + allowance.
        state.sent.push_back((0, start));
        state.acknowledge(0, false, start + Duration::from_millis(5));
        for round in 0..2u64 {
            let at = start + Duration::from_millis(10 + 400 * round);
            state.sent.push_back((round + 1, at));
            assert_eq!(state.admit(at + Duration::from_millis(150)), Gate::Closed);
            state.acknowledge(round + 1, false, at + Duration::from_millis(160));
            // The pump re-admits after every ack; that ends the closure.
            assert!(matches!(
                state.admit(at + Duration::from_millis(161)),
                Gate::Open(_)
            ));
        }
        let Gate::Open(credit) = state.admit(start + Duration::from_millis(600)) else {
            panic!()
        };
        assert!(credit.congested);
        let Gate::Open(credit) = state.admit(start + Duration::from_secs(3)) else {
            panic!()
        };
        assert!(!credit.congested, "congestion decays after the window");
    }

    #[test]
    fn mp10_new_pump_first_frame_is_independent() {
        let pump = DisplayPump::new(7);
        assert_eq!(pump.acknowledge(7, 13, false), Some("running"));
        let now = Instant::now();
        let Gate::Open(credit) = pump.with(|state| state.admit(now)) else {
            panic!()
        };
        assert!(credit.reset, "the viewer may hold no base for this pump");
        let Gate::Open(credit) = pump.with(|state| state.admit(now)) else {
            panic!()
        };
        assert!(!credit.reset);
    }

    #[test]
    fn mp10_foreign_generation_ack_is_refused() {
        let pump = DisplayPump::new(7);
        assert_eq!(pump.acknowledge(8, 1, false), None);
        assert_eq!(pump.acknowledge(7, 1, false), Some("running"));
    }
}
