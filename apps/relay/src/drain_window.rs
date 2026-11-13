//! MP-08 / MP-09 / MP-10 / MP-11: how many bulk bytes a sender keeps
//! unacknowledged on one socket, sized from what the peer's receipts show the
//! path drains. A fixed window queues window/rate seconds of bulk ahead of every
//! small frame on a slow link; this one holds about two round trips of data, so
//! a small frame waits about that long whatever the link and the transfer size.
use crate::frame_transport::CHUNK_BYTES;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// A cold Room reply fits before the first receipt. Once a rate is measured,
/// the adaptive floor is two frames of `chunk_bytes`.
pub const START_WINDOW_BYTES: usize = 64 * 1024;
/// The smallest bulk text frame: its header and its receipt stay a few percent of it.
pub const MIN_CHUNK_BYTES: usize = 4 * 1024;
/// A bulk frame takes about this long on the measured link, so a small frame
/// queued behind one waits about this long (16 KiB take 131 ms at 1 Mbit/s).
const FRAME_TIME: Duration = Duration::from_millis(25);
/// Fills 90 Mbit/s at a 90 ms round trip, and bounds what one socket keeps in
/// flight if the estimate is ever wrong.
pub const CEILING_BYTES: usize = 1024 * 1024;
/// Until the pipe is full, twice what one round trip delivered: the window
/// doubles every round trip.
const STARTUP_GAIN: f64 = 2.0;
/// Once the rate stops growing, half again the measured bandwidth-delay
/// product keeps the pipe full although the window holds whole frames and the
/// minimum round trip is measured on the smallest one (1.25 left a 16 KiB frame
/// of a 60 ms path unsent: 86 % link use), while the queue it builds ahead of a
/// small frame stays under half a round trip (a gain of 2 kept a whole round
/// trip queued: +150 ms at a 150 ms path).
const STEADY_GAIN: f64 = 1.5;
/// The pipe is full when three round trips in a row did not raise the rate by
/// a quarter (BBR's full-pipe test).
const FULL_GROWTH: f64 = 1.25;
const FULL_ROUNDS: u8 = 3;
/// Rate samples older than this are forgotten once a newer one exists, so the
/// window follows a link that slows down (or a peer that shares it) within two
/// seconds of bulk, and an idle connection keeps the pipe it measured.
const RATE_MEMORY: Duration = Duration::from_secs(2);

/// What the sender knew when a frame left: the receipt turns it into a sample.
#[derive(Clone, Copy, Debug)]
pub struct SendStamp {
    sent_at: Instant,
    delivered: u64,
    delivered_at: Instant,
}

/// Delivery-rate and round-trip estimate from receipts (the BBR rate sample:
/// bytes acknowledged between a frame's send and its receipt, over that time).
#[derive(Debug)]
pub struct DrainWindow {
    delivered: u64,
    delivered_at: Instant,
    /// The lowest round trip of this connection. A windowed minimum would
    /// expire into a sample that includes the queue the window itself builds,
    /// and grow the window from its own queue.
    min_rtt: Option<Duration>,
    rates: VecDeque<(Instant, f64)>,
    /// Delivered bytes at which the current round trip ends.
    round_end: u64,
    /// The rate at the last round trip that raised it by a quarter.
    full_rate: f64,
    flat_rounds: u8,
    filled: bool,
    /// The largest bulk frame acknowledged: the window always holds two of them,
    /// so a sender of whole chunks keeps one in flight while the next is answered.
    frame: usize,
    /// When a bulk frame last waited for the window. A frame sent before that
    /// measured the link; one sent while the application had nothing more to
    /// send measured the application.
    blocked_at: Option<Instant>,
    /// Whether a bulk frame waited for the window during the current round trip.
    round_blocked: bool,
}

impl DrainWindow {
    pub fn new(now: Instant) -> Self {
        Self {
            delivered: 0,
            delivered_at: now,
            min_rtt: None,
            rates: VecDeque::new(),
            round_end: 0,
            full_rate: 0.0,
            flat_rounds: 0,
            filled: false,
            frame: 0,
            blocked_at: None,
            round_blocked: false,
        }
    }

    /// The size of the next bulk text frame: about FRAME_TIME at the measured
    /// rate, within MIN_CHUNK_BYTES..=CHUNK_BYTES. Start short so the minimum
    /// round trip does not permanently include a full frame of drain time.
    pub fn chunk_bytes(&self) -> usize {
        self.max_rate().map_or(MIN_CHUNK_BYTES, |rate| {
            ((rate * FRAME_TIME.as_secs_f64()) as usize).clamp(MIN_CHUNK_BYTES, CHUNK_BYTES)
        })
    }

    /// The bulk bytes allowed unacknowledged now. A sender always may write one
    /// frame when nothing is unacknowledged, whatever its size.
    pub fn bytes(&self) -> usize {
        let floor = 2 * self.chunk_bytes().max(self.frame);
        let (Some(rtt), Some(rate)) = (self.min_rtt, self.max_rate()) else {
            return START_WINDOW_BYTES;
        };
        let gain = if self.filled {
            STEADY_GAIN
        } else {
            STARTUP_GAIN
        };
        ((rate * rtt.as_secs_f64() * gain).round() as usize).clamp(floor, CEILING_BYTES)
    }

    /// A bulk frame is waiting because the window is full: the rate measured
    /// from here on is the link's, not what the application had to send.
    pub fn blocked(&mut self, now: Instant) {
        self.blocked_at = Some(now);
        self.round_blocked = true;
    }

    /// Stamp a frame as it is written. `idle` is true when nothing was
    /// unacknowledged, so the time the link sat idle is not counted as drain time.
    pub fn sent(&mut self, now: Instant, idle: bool) -> SendStamp {
        if idle {
            self.delivered_at = now;
        }
        SendStamp {
            sent_at: now,
            delivered: self.delivered,
            delivered_at: self.delivered_at,
        }
    }

    /// The receipt for a frame of `bytes` stamped `stamp`. Only bulk frames
    /// sample the rate: small frames alone never fill the pipe, so their rate
    /// is what the application sent, not what the link can carry.
    pub fn received(&mut self, now: Instant, bytes: usize, stamp: SendStamp, bulk: bool) {
        if bulk {
            self.frame = self.frame.max(bytes).min(CHUNK_BYTES);
        }
        self.delivered += bytes as u64;
        self.delivered_at = now;
        let rtt = now.saturating_duration_since(stamp.sent_at);
        if !rtt.is_zero() {
            self.min_rtt = Some(self.min_rtt.map_or(rtt, |min| min.min(rtt)));
        }
        let interval = now.saturating_duration_since(stamp.delivered_at);
        if bulk && !interval.is_zero() {
            let rate = (self.delivered - stamp.delivered) as f64 / interval.as_secs_f64();
            // A frame the application sent alone measured how little it had to
            // send: it may raise the estimate but never replaces a measured link.
            let window_limited = self.blocked_at.is_some_and(|at| at >= stamp.sent_at);
            if !window_limited && self.max_rate().is_some_and(|max| rate < max) {
                return self.end_round_after(stamp);
            }
            while self
                .rates
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) > RATE_MEMORY)
            {
                // The new sample below replaces it.
                self.rates.pop_front();
            }
            // Keep only samples that can still be the maximum.
            while self.rates.back().is_some_and(|(_, r)| *r <= rate) {
                self.rates.pop_back();
            }
            self.rates.push_back((now, rate));
        }
        if bulk {
            self.end_round_after(stamp);
        }
    }

    /// A round trip ends when a frame sent after the previous end is acknowledged.
    fn end_round_after(&mut self, stamp: SendStamp) {
        if stamp.delivered >= self.round_end {
            self.round_end = self.delivered;
            self.end_round();
        }
    }

    fn end_round(&mut self) {
        // Only a round trip in which the window held frames back can show that
        // the pipe is full; a quiet one shows only the application.
        let blocked = std::mem::take(&mut self.round_blocked);
        let Some(rate) = self.max_rate().filter(|_| !self.filled && blocked) else {
            return;
        };
        if rate >= self.full_rate * FULL_GROWTH {
            self.full_rate = rate;
            self.flat_rounds = 0;
        } else {
            self.flat_rounds += 1;
            self.filled = self.flat_rounds >= FULL_ROUNDS;
        }
    }

    fn max_rate(&self) -> Option<f64> {
        self.rates.front().map(|(_, rate)| *rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_window_starts_at_the_floor_and_doubles_with_each_round_trip() {
        let mut window = DrainWindow::new(Instant::now());
        assert_eq!(window.bytes(), START_WINDOW_BYTES);
        // A fast 90 ms path: everything sent in one round trip comes back in the next.
        for round in 0..4 {
            let in_flight = window.bytes();
            let stamps: Vec<_> = (0..in_flight / CHUNK_BYTES)
                .map(|n| window.sent(Instant::now(), n == 0))
                .collect();
            tokio::time::advance(Duration::from_millis(90)).await;
            for stamp in stamps {
                window.received(Instant::now(), CHUNK_BYTES, stamp, true);
            }
            assert_eq!(
                window.bytes(),
                (2 * in_flight).min(CEILING_BYTES),
                "round {round}"
            );
        }
    }

    /// Drives the window over a simulated bottleneck of `rate` bytes/s and round trip
    /// `rtt` for `seconds`, and returns (link use, mean wait of a small frame behind
    /// queued bulk at the bottleneck) over the second half.
    fn simulate(rate: f64, rtt: Duration, seconds: u64) -> (f64, Duration) {
        simulate_link(rate, rtt, Duration::ZERO, None, Duration::ZERO, seconds)
    }

    /// `simulate`, where the receiver answers each frame `receipt_delay` after it
    /// arrived, the sender writes frames of `frame` bytes (a receiver that takes
    /// only whole chunks) instead of `chunk_bytes`, and for the first `quiet` the
    /// application has only one frame every 400 ms to send (an idle page). The
    /// second half of the time after `quiet` is measured.
    fn simulate_link(
        rate: f64,
        rtt: Duration,
        receipt_delay: Duration,
        frame: Option<usize>,
        quiet: Duration,
        seconds: u64,
    ) -> (f64, Duration) {
        let start = Instant::now();
        let mut window = DrainWindow::new(start);
        let mut now = start;
        while now < start + quiet {
            let frame = frame.unwrap_or_else(|| window.chunk_bytes());
            // One small frame (its receipt sets the minimum round trip) and one bulk frame.
            let small = window.sent(now, true);
            window.received(now + rtt + receipt_delay, 64, small, false);
            let stamp = window.sent(now, true);
            let receipt = now + Duration::from_secs_f64(frame as f64 / rate) + rtt + receipt_delay;
            window.received(receipt, frame, stamp, true);
            now += Duration::from_millis(400);
        }
        let start = now;
        let mut link_free_at = start;
        let mut in_flight: VecDeque<(Instant, usize, SendStamp)> = VecDeque::new();
        let measure_from = start + Duration::from_secs(seconds / 2);
        let (mut sent, mut waits, mut wait_total) = (0usize, 0u32, Duration::ZERO);
        while now < start + Duration::from_secs(seconds) {
            loop {
                let frame = frame.unwrap_or_else(|| window.chunk_bytes());
                let unacknowledged: usize = in_flight.iter().map(|(_, bytes, _)| bytes).sum();
                if unacknowledged > 0 && unacknowledged + frame > window.bytes() {
                    window.blocked(now);
                    break;
                }
                let stamp = window.sent(now, in_flight.is_empty());
                if now >= measure_from {
                    // What a small frame written now waits behind at the bottleneck.
                    wait_total += link_free_at.saturating_duration_since(now);
                    waits += 1;
                    sent += frame;
                }
                link_free_at = link_free_at.max(now) + Duration::from_secs_f64(frame as f64 / rate);
                in_flight.push_back((link_free_at + rtt + receipt_delay, frame, stamp));
            }
            let (receipt_at, frame, stamp) = in_flight.pop_front().unwrap();
            now = receipt_at;
            window.received(now, frame, stamp, true);
        }
        let measured = now.saturating_duration_since(measure_from).as_secs_f64();
        (sent as f64 / (rate * measured), wait_total / waits.max(1))
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayproof r3): at 8 Mbit/s and a 150 ms round trip the
    // link stays full while a small frame waits well under half a round trip behind bulk;
    // a gain of 2 kept a whole round trip (150 ms) queued ahead of it.
    #[tokio::test(start_paused = true)]
    async fn the_window_fills_a_long_path_with_a_short_standing_queue() {
        for (rate, rtt) in [(1_000_000.0, 150), (1_000_000.0, 60), (125_000.0, 150)] {
            let rtt = Duration::from_millis(rtt);
            let (link_use, wait) = simulate(rate, rtt, 30);
            assert!(link_use >= 0.95, "{rate} B/s, {rtt:?}: link use {link_use}");
            assert!(
                wait <= rtt / 2 + 2 * FRAME_TIME,
                "{rate} B/s, {rtt:?}: a small frame waits {wait:?} behind bulk"
            );
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (transport3): a browser viewer takes only whole 16 KiB
    // binary chunks and answers each one after its own event loop turn. The window was sized
    // from the round trip of the smallest frame, so it never held two whole chunks and the
    // display reached the viewer at one chunk per round trip (1.6 Mbit/s of an 8 Mbit/s link).
    #[tokio::test(start_paused = true)]
    async fn whole_chunks_with_slow_receipts_fill_the_link() {
        for (rate, rtt, delay) in [
            (1_000_000.0, 60, 20),
            (1_000_000.0, 60, 0),
            (1_000_000.0, 150, 20),
            (250_000.0, 60, 20),
        ] {
            let (rtt, delay) = (Duration::from_millis(rtt), Duration::from_millis(delay));
            let (link_use, wait) =
                simulate_link(rate, rtt, delay, Some(CHUNK_BYTES), Duration::from_secs(5), 30);
            // Whole 16 KiB frames are 65 ms each at 2 Mbit/s: the last one of a round
            // trip leaves the link idle a little longer than short frames do.
            assert!(
                link_use >= 0.85,
                "{rate} B/s, {rtt:?} + {delay:?}: link use {link_use}"
            );
            assert!(
                wait <= (rtt + delay) / 2 + Duration::from_secs_f64(2.0 * CHUNK_BYTES as f64 / rate),
                "{rate} B/s, {rtt:?} + {delay:?}: a small frame waits {wait:?} behind bulk"
            );
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (transport3): adaptive frames whose receipts come back later
    // than the minimum round trip still fill the link.
    #[tokio::test(start_paused = true)]
    async fn short_frames_with_slow_receipts_fill_the_link() {
        let (link_use, _) = simulate_link(
            1_000_000.0,
            Duration::from_millis(60),
            Duration::from_millis(40),
            None,
            Duration::from_secs(5),
            30,
        );
        assert!(link_use >= 0.9, "link use {link_use}");
    }
}
