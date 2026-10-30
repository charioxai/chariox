//! MP-08/MP-10: protocol 491 pushed display. One kernel-resident pump per
//! relay display subscription takes frames through the ordinary admitted
//! display path (terminal admission, Vault barrier, host policy, scrub) and
//! pushes them as relay events. The relay RTT leaves the frame loop; the
//! client's acknowledgements gate it, after Selkies' frame ACK backpressure
//! (selkies websockets_mode.py `_run_frame_backpressure_logic`, MPL-2.0).
use super::*;
use crate::runtime::browser_controller_process::{DirectFrame, DirectFrames};
use crate::runtime::state::{KernelBrowserDisplayRequest, KernelBrowserPushCredit};
use std::collections::VecDeque;

/// Legacy clients have no receiver hold clock. Timed acknowledgements use
/// an observed frame interval plus RTT jitter instead of this allowance.
const GATE_ALLOWANCE: Duration = Duration::from_millis(100);
/// Selkies STALLED_CLIENT_TIMEOUT_SECONDS / STALLED_CLIENT_REPROBE_SECONDS.
const STALL: Duration = Duration::from_secs(4);
const REPROBE: Duration = Duration::from_secs(2);
const RTT_WINDOW: Duration = Duration::from_secs(10);
/// Two gate closures within this window lower the motion bitrate.
const CONGESTION_WINDOW: Duration = Duration::from_secs(2);
/// Consecutive host failures before the pump stops and the client resubscribes.
const FAILURE_LIMIT: u32 = 5;
/// Stale-document refusals that last this long mean the bound document is gone for good.
const STALE_LIMIT: Duration = Duration::from_millis(300);
/// Selkies VIDEO_RELAY_SYNC_FLOOR_SECONDS: at most one key request per second.
const SYNC_FLOOR: Duration = Duration::from_secs(1);

#[derive(Default)]
struct PumpState {
    /// Tail of the display id, labelling this pump's timing marks.
    display: String,
    generation: u64,
    started: bool,
    stopped: bool,
    failed: bool,
    acked: u64,
    reset: bool,
    sent: VecDeque<(u64, Instant)>,
    rtt: VecDeque<(Instant, Duration)>,
    timed_ack: bool,
    last_frame_at: Option<Instant>,
    frame_interval: Option<Duration>,
    gated_at: Option<Instant>,
    /// Gate closures in the congestion window, with the oldest unacked frame that closed each.
    closes: VecDeque<(Instant, u64)>,
    synced_at: Option<Instant>,
    /// Nobody watches: no capture and no reprobe until an acknowledgement asks for an independent frame.
    paused: bool,
    /// Paused acknowledgements seen, and how many of them the host display has been told about.
    pauses: u64,
    host: HostView,
}

/// What the host display was last told: its source reads nothing while parked, and each hold renews its 60 s expiry.
#[derive(Default)]
struct HostView {
    parked: bool,
    pauses: u64,
}

/// What the host display needs next to follow the pump.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HostPark {
    parked: bool,
    pauses: u64,
}

#[derive(Debug, PartialEq)]
enum Gate {
    Closed,
    Open(KernelBrowserPushCredit),
}

impl PumpState {
    fn mark(&self, stage: &'static str, started: Instant) {
        crate::transport::kernel_browser_display::timing_for(stage, Some(&self.display), started);
    }
    fn rtt_floor(&self) -> Duration {
        self.rtt
            .iter()
            .map(|(_, rtt)| *rtt)
            .min()
            .unwrap_or_default()
    }
    // MP-08/MP-10/MP-11: sparse idle gaps are not frame intervals. Observe
    // the source cadence while it is moving; bootstrap at 30fps until learned.
    fn sent_frame(&mut self, sequence: u64, now: Instant) {
        if let Some(previous) = self.last_frame_at {
            let interval = now.saturating_duration_since(previous);
            if interval > Duration::ZERO && interval < Duration::from_millis(250) {
                let sample = interval.max(Duration::from_millis(8));
                self.frame_interval = Some(
                    self.frame_interval
                        .map_or(sample, |old| (old * 3 + sample) / 4),
                );
            }
        }
        self.last_frame_at = Some(now);
        self.sent.push_back((sequence, now));
    }
    fn gate_allowance(&self) -> Duration {
        if !self.timed_ack || self.rtt.is_empty() {
            return GATE_ALLOWANCE;
        }
        let mut samples: Vec<_> = self.rtt.iter().map(|(_, sample)| *sample).collect();
        samples.sort_unstable();
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        let jitter = p95.saturating_sub(self.rtt_floor());
        self.frame_interval
            .unwrap_or(Duration::from_nanos(1_000_000_000 / 30))
            + jitter
            + Duration::from_millis(5)
    }
    fn admit(&mut self, now: Instant) -> Gate {
        if self.paused {
            return Gate::Closed;
        }
        // MP-08/MP-10/MP-11: private fixed-label timings, no peer or payload.
        self.mark(
            "push_rtt_floor",
            now.checked_sub(self.rtt_floor()).unwrap_or(now),
        );
        if let Some(&(oldest_sequence, oldest)) = self.sent.front() {
            let age = now.saturating_duration_since(oldest);
            if age > STALL {
                // A stalled gate is sent nothing; reopen on an independent
                // frame after the reprobe interval (Selkies semantics).
                if self.gated_at.is_none() {
                    self.mark("push_gate_closed", oldest);
                }
                let gated = *self.gated_at.get_or_insert(now);
                if now.saturating_duration_since(gated) < REPROBE {
                    return Gate::Closed;
                }
                self.sent.clear();
                self.reset = true;
            } else if age > self.rtt_floor() + self.gate_allowance() {
                self.mark("push_oldest_unacked", oldest);
                if self.gated_at.is_none() {
                    self.mark("push_gate_closed", oldest);
                    self.gated_at = Some(now);
                    self.closes.push_back((now, oldest_sequence));
                }
                return Gate::Closed;
            }
        }
        if let Some(closed_at) = self.gated_at {
            self.mark("push_gate_reopened", closed_at);
        }
        self.gated_at = None;
        while self
            .closes
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > CONGESTION_WINDOW)
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
    /// Nobody watches: forget what is in flight instead of letting the stall gate reprobe, and resume from an
    /// independent frame.
    fn pause(&mut self) {
        self.started = true;
        self.paused = true;
        self.pauses += 1;
        self.sent.clear();
        self.gated_at = None;
        self.reset = true;
    }
    /// A paused pump parks the host display on its first paused acknowledgement and again on each later one (the
    /// host expires an unused display after 60 s); the independent-frame request that resumes it releases the host first.
    fn next_host_park(&self) -> Option<HostPark> {
        let wanted = HostPark {
            parked: self.paused,
            pauses: self.pauses,
        };
        let told = HostPark {
            parked: self.host.parked,
            pauses: self.host.pauses,
        };
        (wanted != told && (self.paused || self.host.parked)).then_some(wanted)
    }
    fn host_told(&mut self, park: HostPark) {
        self.host = HostView {
            parked: park.parked,
            pauses: park.pauses,
        };
    }
    fn acknowledge(&mut self, sequence: u64, lost: bool, now: Instant, held_ms: Option<u32>) {
        self.started = true;
        if self.paused {
            // An acknowledgement of a frame sent before the pause may arrive after it: only the viewer asking
            // for an independent frame (watched again) resumes.
            if !lost {
                return;
            }
            self.paused = false;
        }
        self.reset |= lost;
        self.acked = self.acked.max(sequence);
        while let Some(&(sent, at)) = self.sent.front() {
            if sent > sequence {
                break;
            }
            self.sent.pop_front();
            if sent == sequence {
                self.mark("push_frame_ack", at);
                let age = now.saturating_duration_since(at);
                let sample = match held_ms {
                    Some(held) => age.checked_sub(Duration::from_millis(u64::from(held))),
                    None if !self.timed_ack => Some(age),
                    None => None,
                };
                if let Some(sample) = sample {
                    if held_ms.is_some() {
                        // Old, untimed samples include decoder/ACK hold. Once
                        // the receiver supplies a clock, use only wire samples.
                        if !self.timed_ack {
                            self.rtt.clear();
                        }
                        self.timed_ack = true;
                        self.mark(
                            "push_ack_rtt",
                            Instant::now().checked_sub(sample).unwrap_or(now),
                        );
                    }
                    // MP-08/MP-10: a closure whose frame then crossed the wire within the
                    // allowance was the receiver's own decode/present hold (a navigation's
                    // tiles and mirror on the viewer's main thread), not congestion: it must
                    // not lower the motion bitrate. Untimed ACKs keep every closure.
                    if held_ms.is_some()
                        && !self.rtt.is_empty()
                        && sample <= self.rtt_floor() + self.gate_allowance()
                    {
                        self.closes.retain(|&(_, closed)| closed != sequence);
                    }
                    self.rtt.push_back((now, sample));
                }
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
    delivery: super::display_delivery::DisplayTask,
}

impl DisplayPump {
    /// A new pump (also a re-subscription after a dropped relay socket, whose
    /// in-flight frames were lost) starts with an independent frame.
    pub(super) fn new(generation: u64, display_id: &str) -> Arc<Self> {
        Arc::new(Self {
            state: std::sync::Mutex::new(PumpState {
                display: display_id
                    .chars()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
                generation,
                reset: true,
                ..Default::default()
            }),
            wake: tokio::sync::Notify::new(),
            delivery: Default::default(),
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
        held_ms: Option<u32>,
        paused: bool,
    ) -> Option<&'static str> {
        let status = self.with(|state| {
            if state.generation != generation {
                return None;
            }
            if paused {
                state.pause();
            } else {
                state.acknowledge(sequence, lost, Instant::now(), held_ms);
            }
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
        self.delivery.stop();
    }
    pub(super) fn start_delivery(
        &self,
        delivery: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        self.delivery.start(delivery);
        if self.with(|state| state.stopped) {
            self.delivery.stop();
        }
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

/// What a failed capture credit means for the pump.
#[derive(Default)]
struct Faults {
    failures: u32,
    stale_since: Option<Instant>,
}

enum Next {
    Fail,
    Retry { delay: Duration, reset: bool },
}

impl Faults {
    fn error(&mut self, error: &DaemonError, now: Instant) -> Next {
        match error {
            // A navigation between capture and commit refuses one credit as a stale document and
            // the next credit binds the new one. Staleness that persists is a retired source: only
            // the client can rebind it, so end the pump now instead of after the failure backoff.
            DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::StaleReference,
            } => {
                let since = *self.stale_since.get_or_insert(now);
                if now.duration_since(since) >= STALE_LIMIT {
                    Next::Fail
                } else {
                    Next::Retry {
                        delay: Duration::from_millis(10),
                        reset: false,
                    }
                }
            }
            // A gone subscription (stale generation, revoked grant) is final; other host failures
            // retry on an independent frame.
            DaemonError::UserDomainRefused { .. } => Next::Fail,
            _ => {
                self.failures += 1;
                if self.failures >= FAILURE_LIMIT {
                    Next::Fail
                } else {
                    Next::Retry {
                        delay: Duration::from_millis(250) * self.failures,
                        reset: true,
                    }
                }
            }
        }
    }
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
    crate::transport::kernel_browser_display::timing("push_gate_started", Instant::now());
    let daemon_private_key = route.router.relay_private_key();
    let mut last_sent = pump.with(|state| state.acked);
    let (mut idle, mut faults) = (Duration::ZERO, Faults::default());
    let mut direct = DirectLease(None);
    loop {
        if pump.with(|state| state.stopped) || !lease_alive(&route).await {
            return;
        }
        if let Some(park) = pump.with(|state| state.next_host_park()) {
            if park.parked {
                direct.end();
            }
            match park_host(&route, generation, park.parked).await {
                Ok(()) => {
                    faults = Faults::default();
                    pump.with(|state| state.host_told(park));
                }
                Err(error) => match faults.error(&error, Instant::now()) {
                    Next::Fail => {
                        crate::transport::kernel_browser_display::pump_failed(&error);
                        pump.with(|state| state.failed = true);
                        return;
                    }
                    Next::Retry { delay, .. } => sleep(delay).await,
                },
            }
            continue;
        }
        let credit = match pump.with(|state| state.admit(Instant::now())) {
            Gate::Closed => {
                let _ = timeout(Duration::from_millis(20), pump.wake.notified()).await;
                continue;
            }
            Gate::Open(credit) => credit,
        };
        // L3: while Node's lease lasts, whole frames come straight from the worker. A
        // reset or congestion ends it and this credit goes to Node, which resumes on a key.
        if let Some((frames, token)) = &direct.0 {
            if credit.reset || credit.congested {
                direct.end();
            } else {
                match frames
                    .next(token, &route.display_id, Duration::from_millis(100))
                    .await
                {
                    DirectFrame::Idle => continue,
                    DirectFrame::Ended => direct.0 = None,
                    DirectFrame::Frame(frame, revision) => {
                        let Some(sequence) = frame["sequence"].as_u64() else {
                            direct.end();
                            continue;
                        };
                        let Ok(encrypted_event) = seal(&route, &daemon_private_key, frame) else {
                            direct.end();
                            continue;
                        };
                        frames.delivered(token, revision);
                        pump.with(|state| state.sent_frame(sequence, Instant::now()));
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
                        continue;
                    }
                }
            }
        }
        let started = Instant::now();
        let result =
            capture_frame(&route, &daemon_private_key, generation, last_sent, credit).await;
        let captured = match result {
            Ok(result) => {
                faults = Faults::default();
                result
            }
            Err(error) => match faults.error(&error, Instant::now()) {
                Next::Fail => {
                    crate::transport::kernel_browser_display::pump_failed(&error);
                    pump.with(|state| state.failed = true);
                    return;
                }
                Next::Retry { delay, reset } => {
                    if reset {
                        pump.with(|state| state.reset = true);
                    }
                    sleep(delay).await;
                    continue;
                }
            },
        };
        let Some((sequence, encrypted_event, lease)) = captured else {
            if started.elapsed() < Duration::from_millis(5) {
                idle = (idle * 2).clamp(Duration::from_millis(8), Duration::from_millis(100));
                let _ = timeout(idle, pump.wake.notified()).await;
            }
            continue;
        };
        idle = Duration::ZERO;
        let at = std::time::Instant::now();
        pump.with(|state| state.sent_frame(sequence, Instant::now()));
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
        if let Some((token, generation)) = lease {
            let state = route.router.runtime_state();
            if let Some(frames) = state.kernel_browser_direct_frames(&route.caller) {
                if frames.bind(&token, &route.display_id, generation, sequence) {
                    direct.0 = Some((frames, token));
                }
            }
        }
    }
}

/// L3: the bound worker lease; ending it (or dropping the pump) retires its generation.
struct DirectLease(Option<(Arc<DirectFrames>, String)>);
impl DirectLease {
    fn end(&mut self) {
        if let Some((frames, token)) = self.0.take() {
            frames.retire(&token);
        }
    }
}
impl Drop for DirectLease {
    fn drop(&mut self) {
        self.end();
    }
}

fn seal(
    route: &PumpRoute,
    private_key: &str,
    frame: serde_json::Value,
) -> Result<chariox_relay::protocol::EncryptedRelayPayload, DaemonError> {
    let failed = |message: &str| DaemonError::LocalTransport {
        operation: "display frame",
        message: message.into(),
    };
    let event = KernelEvent::KernelBrowserFrame {
        subscription_id: route.display_id.clone(),
        frame,
    };
    let bytes = crate::transport::kernel_browser_display::encode_display_event(event)
        .map_err(|_| failed("invalid display frame"))?;
    relay_crypto::encrypt_payload_for_peer(private_key, &route.public_key, &bytes)
        .map_err(|_| failed("display encryption failed"))
}

/// An unwatched display reads nothing back from its source and keeps its host binding alive without a capture.
async fn park_host(route: &PumpRoute, generation: u64, parked: bool) -> Result<(), DaemonError> {
    let state = route.router.runtime_state();
    if state.is_room_display(&route.display_id) {
        return Ok(());
    }
    state
        .kernel_browser_display_request(
            &route.caller,
            KernelBrowserDisplayRequest::Park {
                subscription_id: route.display_id.clone(),
                generation,
                parked,
                watch: true,
            },
        )
        .await
        .map(|_| ())
}

// One ACK gate and event transport; Room placement supplies worker-sealed CXD1
// rather than admitting a viewer as a terminal on the worker (MP-08/MP-11).
type Captured = (
    u64,
    chariox_relay::protocol::EncryptedRelayPayload,
    Option<(String, u64)>,
);
async fn capture_frame(
    route: &PumpRoute,
    private_key: &str,
    generation: u64,
    after_sequence: u64,
    credit: KernelBrowserPushCredit,
) -> Result<Option<Captured>, DaemonError> {
    let state = route.router.runtime_state();
    if state.is_room_display(&route.display_id) {
        return Ok(state
            .room_display_frame(
                &route.caller.caller,
                &route.display_id,
                generation,
                after_sequence,
                credit.reset,
                credit.congested,
            )
            .await?
            .map(|frame| (frame.sequence, frame.encrypted_event, None)));
    }
    let result = state
        .kernel_browser_display_request(
            &route.caller,
            KernelBrowserDisplayRequest::EncodedCapture {
                subscription_id: route.display_id.clone(),
                generation,
                after_sequence,
                push: Some(credit),
            },
        )
        .await?;
    let mut response = serde_json::json!({"KernelBrowser":{"result":result}});
    // L3: a private lease grant Node returns with this frame; never forwarded.
    let lease = response
        .pointer_mut("/KernelBrowser/result/direct_lease")
        .map(serde_json::Value::take)
        .and_then(|lease| {
            let token = lease["token"].as_str()?;
            let generation = lease["generation"].as_u64()?;
            (token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| (token.to_owned(), generation))
        });
    let Some((_, sequence, event)) =
        crate::transport::kernel_browser_display::take_display_event(&mut response)
    else {
        return Ok(None);
    };
    let at = std::time::Instant::now();
    let KernelEvent::KernelBrowserFrame { frame, .. } = event else {
        return Ok(None);
    };
    let encrypted = seal(route, private_key, frame)?;
    crate::transport::kernel_browser_display::timing("event_serialize_encrypt", at);
    Ok(Some((sequence, encrypted, lease)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MP-08/MP-10/MP-11: real gate state and opt-in trace. The evidence parser
    // distinguishes closure transitions from repeated closed polling.
    #[test]
    fn retired_source_ends_the_pump_at_once_but_one_stale_credit_does_not() {
        use crate::error::UserDomainRefusalReason::{NotGranted, StaleReference};
        let stale = DaemonError::UserDomainRefused {
            reason: StaleReference,
        };
        let t0 = Instant::now();
        let mut faults = Faults::default();
        assert!(matches!(
            faults.error(&stale, t0),
            Next::Retry { reset: false, .. }
        ));
        assert!(matches!(
            faults.error(&stale, t0 + STALE_LIMIT / 2),
            Next::Retry { .. }
        ));
        assert!(matches!(faults.error(&stale, t0 + STALE_LIMIT), Next::Fail));
        // A delivered credit forgets the stale run.
        assert!(matches!(
            Faults::default().error(&stale, t0 + STALE_LIMIT),
            Next::Retry { .. }
        ));
        let refused = DaemonError::UserDomainRefused { reason: NotGranted };
        assert!(matches!(Faults::default().error(&refused, t0), Next::Fail));
    }

    #[test]
    fn other_host_failures_retry_with_a_reset_until_the_limit() {
        let error = DaemonError::LocalTransport {
            operation: "kernel_browser",
            message: "x".into(),
        };
        let mut faults = Faults::default();
        for _ in 1..FAILURE_LIMIT {
            assert!(matches!(
                faults.error(&error, Instant::now()),
                Next::Retry { reset: true, .. }
            ));
        }
        assert!(matches!(faults.error(&error, Instant::now()), Next::Fail));
    }

    #[test]
    fn mp10_gate_trace_observation() {
        let now = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(0, false, now, None);
        assert!(matches!(state.admit(now), Gate::Open(_)));
        state.sent.push_back((1, now - Duration::from_millis(200)));
        assert_eq!(state.admit(now), Gate::Closed);
        assert_eq!(state.admit(now), Gate::Closed);
        state.acknowledge(1, false, now, None);
        assert!(matches!(state.admit(now), Gate::Open(_)));
        state.sent.push_back((2, now - Duration::from_millis(400)));
        assert_eq!(state.admit(now), Gate::Closed);
    }

    // The band and the page are separate displays, each with its own pump: a page
    // gate closed by stale unacked frames never holds a band (keyboard echo) frame.
    #[test]
    fn mp10_band_pump_admits_while_the_page_pump_gate_is_closed() {
        let now = Instant::now();
        let (mut page, mut band) = (PumpState::default(), PumpState::default());
        for state in [&mut page, &mut band] {
            state.acknowledge(0, false, now, None);
        }
        for sequence in 1..=3 {
            page.sent
                .push_back((sequence, now - Duration::from_millis(300)));
        }
        assert_eq!(page.admit(now), Gate::Closed);
        assert!(matches!(band.admit(now), Gate::Open(_)));
        band.sent_frame(1, now);
        assert!(matches!(
            band.admit(now + Duration::from_millis(1)),
            Gate::Open(_)
        ));
    }

    #[test]
    fn mp10_ack_gate_closes_on_lag_and_lifts_after_ack() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(0, false, start, None);
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
        state.acknowledge(3, false, start + Duration::from_millis(160), None);
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
        state.acknowledge(4, true, start, None);
        let Gate::Open(credit) = state.admit(start) else {
            panic!()
        };
        assert!(credit.reset, "lost frame must retire the reference chain");
        let Gate::Open(credit) = state.admit(start) else {
            panic!()
        };
        assert!(!credit.reset, "reset is consumed once");
        state.acknowledge(4, true, start, None);
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
    fn mp08_a_paused_pump_stays_dormant_past_the_reprobe_until_an_independent_frame_is_asked() {
        let start = Instant::now();
        let unwatched = start + STALL + REPROBE + Duration::from_secs(30);
        let mut state = PumpState::default();
        state.acknowledge(0, false, start, None);
        assert!(matches!(state.admit(start), Gate::Open(_)));
        state.sent.push_back((1, start));
        // Without a pause, silence reopens the gate on the reprobe: an animated page keeps producing bursts.
        let mut silent = PumpState::default();
        silent.acknowledge(0, false, start, None);
        silent.sent.push_back((1, start));
        assert_eq!(
            silent.admit(start + STALL + Duration::from_millis(1)),
            Gate::Closed
        );
        assert!(matches!(silent.admit(unwatched), Gate::Open(_)));
        // A paused pump never does, however long nobody watches.
        state.pause();
        assert_eq!(
            state.admit(start + STALL + Duration::from_millis(1)),
            Gate::Closed
        );
        assert_eq!(state.admit(unwatched), Gate::Closed);
        // The late acknowledgement of a frame sent before the pause does not resume it.
        state.acknowledge(1, false, unwatched, Some(0));
        assert_eq!(state.admit(unwatched), Gate::Closed);
        // Watched again: the independent-frame request resumes it with a reset.
        state.acknowledge(1, true, unwatched, None);
        let Gate::Open(credit) = state.admit(unwatched) else {
            panic!("watched again resumes the pump")
        };
        assert!(credit.reset && state.sent.is_empty());
    }

    #[test]
    fn mp08_a_paused_pump_parks_the_host_display_on_every_paused_ack_and_releases_it_when_watched()
    {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(0, false, start, None);
        assert_eq!(
            state.next_host_park(),
            None,
            "a watched pump leaves the host display alone"
        );
        state.pause();
        let park = state
            .next_host_park()
            .expect("the first paused ack parks the host display");
        assert!(park.parked);
        state.host_told(park);
        assert_eq!(state.next_host_park(), None);
        // The host expires an unused display after 60 s: each repeated paused ack renews it.
        state.pause();
        let renew = state
            .next_host_park()
            .expect("a repeated paused ack renews the host display");
        assert!(renew.parked);
        state.host_told(renew);
        assert_eq!(state.next_host_park(), None);
        // Watched again: release the source before the independent frame.
        state.acknowledge(1, true, start, None);
        let release = state
            .next_host_park()
            .expect("watching releases the host display");
        assert!(!release.parked);
        state.host_told(release);
        assert_eq!(state.next_host_park(), None);
    }

    #[test]
    fn mp10_repeated_gate_closure_reports_congestion() {
        let start = Instant::now();
        let mut state = PumpState::default();
        // A 5 ms RTT floor: 150 ms of lag exceeds floor + allowance.
        state.sent.push_back((0, start));
        state.acknowledge(0, false, start + Duration::from_millis(5), None);
        for round in 0..2u64 {
            let at = start + Duration::from_millis(10 + 400 * round);
            state.sent.push_back((round + 1, at));
            assert_eq!(state.admit(at + Duration::from_millis(150)), Gate::Closed);
            state.acknowledge(round + 1, false, at + Duration::from_millis(160), None);
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

    // browserperf r4: a viewer busy presenting a navigation (late ACK, short wire time) is not congested.
    #[test]
    fn mp10_receiver_hold_closures_do_not_report_congestion() {
        // Two closures 0.9 s apart over an 80 ms wire floor; `wire` decides where each ACK's delay was.
        let run = |wire: bool| {
            let start = Instant::now();
            let mut state = PumpState::default();
            state.sent_frame(0, start);
            state.acknowledge(0, false, start + Duration::from_millis(80), Some(0));
            let mut congested = false;
            for (round, delay) in [(1u64, 260u64), (2, 700)] {
                let at = start + Duration::from_millis(100 + 900 * (round - 1));
                state.sent_frame(round, at);
                assert_eq!(
                    state.admit(at + Duration::from_millis(delay - 10)),
                    Gate::Closed
                );
                let held = if wire { 0 } else { delay as u32 - 80 };
                state.acknowledge(round, false, at + Duration::from_millis(delay), Some(held));
                let Gate::Open(credit) = state.admit(at + Duration::from_millis(delay + 1)) else {
                    panic!("the ACK reopens the gate")
                };
                congested |= credit.congested;
            }
            congested
        };
        assert!(
            !run(false),
            "a viewer holding frames over an 80 ms wire is not congested"
        );
        assert!(run(true), "the same delays on the wire are congestion");
    }

    #[test]
    fn mp10_new_pump_first_frame_is_independent() {
        let pump = DisplayPump::new(7, "display");
        assert_eq!(pump.acknowledge(7, 13, false, None, false), Some("running"));
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
        let pump = DisplayPump::new(7, "display");
        assert_eq!(pump.acknowledge(8, 1, false, None, false), None);
        assert_eq!(pump.acknowledge(7, 1, false, None, false), Some("running"));
    }
    // MP-08/MP-10/MP-11: receiver-side decode/queue hold is not wire RTT.
    #[test]
    fn mp10_timed_ack_excludes_receiver_hold() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.sent.push_back((1, start));
        state.acknowledge(1, false, start + Duration::from_millis(120), Some(30));
        assert_eq!(state.rtt_floor(), Duration::from_millis(90));
    }

    #[test]
    fn mp10_timed_ack_gate_uses_frame_interval_instead_of_fixed_100ms() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.sent.push_back((1, start));
        state.acknowledge(1, false, start + Duration::from_millis(90), Some(0));
        state
            .sent
            .push_back((2, start + Duration::from_millis(100)));
        assert_eq!(
            state.admit(start + Duration::from_millis(240)),
            Gate::Closed
        );
    }

    #[test]
    fn mp11_invalid_receiver_hold_does_not_fabricate_zero_rtt() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.sent.push_back((1, start));
        state.acknowledge(1, false, start + Duration::from_millis(90), Some(0));
        state
            .sent
            .push_back((2, start + Duration::from_millis(100)));
        state.acknowledge(2, false, start + Duration::from_millis(110), Some(100));
        assert_eq!(state.rtt_floor(), Duration::from_millis(90));
        assert!(
            state.sent.is_empty(),
            "invalid telemetry must not strand an admitted ACK"
        );
    }
    #[test]
    fn mp10_timed_gate_allows_observed_wire_jitter() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.sent_frame(1, start);
        state.acknowledge(1, false, start + Duration::from_millis(90), Some(0));
        state.sent_frame(2, start + Duration::from_millis(100));
        state.acknowledge(2, false, start + Duration::from_millis(220), Some(0));
        state.frame_interval = Some(Duration::from_millis(33));
        state
            .sent
            .push_back((3, start + Duration::from_millis(300)));
        assert!(
            matches!(
                state.admit(start + Duration::from_millis(440)),
                Gate::Open(_)
            ),
            "a 30ms wire RTT excursion must not close a healthy 30fps lane"
        );
    }

    #[test]
    fn mp10_timed_gate_does_not_treat_idle_gaps_as_frame_intervals() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.sent_frame(1, start);
        state.acknowledge(1, false, start + Duration::from_millis(90), Some(0));
        state.sent_frame(2, start + Duration::from_secs(10));
        assert!(state.gate_allowance() < Duration::from_millis(50));
    }

    #[test]
    fn mp10_30fps_90ms_wire_rtt_with_receiver_hold_never_closes_the_gate() {
        let start = Instant::now();
        let mut state = PumpState::default();
        state.acknowledge(0, false, start, None);
        let mut arrivals = VecDeque::new();
        let mut closures = 0;
        let mut sequence = 0;
        for ms in 0..2000u64 {
            let now = start + Duration::from_millis(ms);
            while arrivals.front().is_some_and(|(at, _)| *at <= ms) {
                let (_, sequence) = arrivals.pop_front().unwrap();
                state.acknowledge(sequence, false, now, Some(5));
            }
            if matches!(state.admit(now), Gate::Closed) {
                closures += 1;
            } else if ms % 33 == 0 {
                sequence += 1;
                state.sent_frame(sequence, now);
                arrivals.push_back((ms + 95, sequence));
            }
        }
        assert_eq!(closures, 0);
        assert_eq!(state.rtt_floor(), Duration::from_millis(90));
    }
}
