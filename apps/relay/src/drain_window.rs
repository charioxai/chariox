//! MP-08 / MP-09 / MP-10 / MP-11: how many bulk bytes a sender keeps
//! unacknowledged on one socket, sized from what the peer's receipts show the
//! path drains. A fixed window queues window/rate seconds of bulk ahead of every
//! small frame on a slow link; this one holds about two round trips of data, so
//! a small frame waits about that long whatever the link and the transfer size.
use crate::frame_transport::CHUNK_BYTES;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// Two chunks before a rate is measured: one drains while the next already
/// waits behind it, so a slow link never idles for a receipt. Once the rate is
/// known the floor is two frames of `chunk_bytes`.
pub const FLOOR_BYTES: usize = 2 * CHUNK_BYTES;
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
        }
    }

    /// The size of the next bulk text frame: about FRAME_TIME at the measured
    /// rate, within MIN_CHUNK_BYTES..=CHUNK_BYTES; CHUNK_BYTES until a rate is known.
    pub fn chunk_bytes(&self) -> usize {
        self.max_rate().map_or(CHUNK_BYTES, |rate| {
            ((rate * FRAME_TIME.as_secs_f64()) as usize).clamp(MIN_CHUNK_BYTES, CHUNK_BYTES)
        })
    }

    /// The size of the next bulk frame toward a peer that takes frames of any size: the smallest
    /// until a rate is measured. The lowest round trip is the sticky input of the window, and a
    /// first frame of CHUNK_BYTES would put its own drain time (131 ms at 1 Mbit/s) into it for
    /// good, keeping the window, and the queue ahead of a small frame, about twice as long.
    pub fn first_chunk_bytes(&self) -> usize {
        self.max_rate()
            .map_or(MIN_CHUNK_BYTES, |_| self.chunk_bytes())
    }

    /// The bulk bytes allowed unacknowledged now. A sender always may write one
    /// frame when nothing is unacknowledged, whatever its size.
    pub fn bytes(&self) -> usize {
        let floor = 2 * self.chunk_bytes();
        let (Some(rtt), Some(rate)) = (self.min_rtt, self.max_rate()) else {
            return floor;
        };
        let gain = if self.filled {
            STEADY_GAIN
        } else {
            STARTUP_GAIN
        };
        ((rate * rtt.as_secs_f64() * gain).round() as usize).clamp(floor, CEILING_BYTES)
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
        self.delivered += bytes as u64;
        self.delivered_at = now;
        let rtt = now.saturating_duration_since(stamp.sent_at);
        if !rtt.is_zero() {
            self.min_rtt = Some(self.min_rtt.map_or(rtt, |min| min.min(rtt)));
        }
        let interval = now.saturating_duration_since(stamp.delivered_at);
        if bulk && !interval.is_zero() {
            let rate = (self.delivered - stamp.delivered) as f64 / interval.as_secs_f64();
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
        // A round trip ends when a frame sent after the previous end is acknowledged.
        if bulk && stamp.delivered >= self.round_end {
            self.round_end = self.delivered;
            self.end_round();
        }
    }

    fn end_round(&mut self) {
        let Some(rate) = self.max_rate().filter(|_| !self.filled) else {
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
        assert_eq!(window.bytes(), FLOOR_BYTES);
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
        let start = Instant::now();
        let mut window = DrainWindow::new(start);
        let (mut now, mut link_free_at) = (start, start);
        let mut in_flight: VecDeque<(Instant, usize, SendStamp)> = VecDeque::new();
        let measure_from = start + Duration::from_secs(seconds / 2);
        let (mut sent, mut waits, mut wait_total) = (0usize, 0u32, Duration::ZERO);
        while now < start + Duration::from_secs(seconds) {
            loop {
                let frame = window.chunk_bytes();
                let unacknowledged: usize = in_flight.iter().map(|(_, bytes, _)| bytes).sum();
                if unacknowledged > 0 && unacknowledged + frame > window.bytes() {
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
                in_flight.push_back((link_free_at + rtt, frame, stamp));
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
}
