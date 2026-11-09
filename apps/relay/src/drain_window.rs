//! MP-08 / MP-09 / MP-10 / MP-11: how many bulk bytes a sender keeps
//! unacknowledged on one socket, sized from what the peer's receipts show the
//! path drains. A fixed window queues window/rate seconds of bulk ahead of every
//! small frame on a slow link; this one holds about two round trips of data, so
//! a small frame waits about that long whatever the link and the transfer size.
use crate::frame_transport::CHUNK_BYTES;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// Two chunks: one drains while the next already waits behind it, so a slow
/// link never idles for a receipt, and a small frame waits at most two chunk
/// times behind bulk (0.5 s at 64 KB/s).
pub const FLOOR_BYTES: usize = 2 * CHUNK_BYTES;
/// Fills 90 Mbit/s at a 90 ms round trip, and bounds what one socket keeps in
/// flight if the estimate is ever wrong.
pub const CEILING_BYTES: usize = 1024 * 1024;
/// Twice the measured bandwidth-delay product keeps the pipe full through
/// receipt bursts and rate changes; the queue it can build is one round trip.
const GAIN: f64 = 2.0;
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
}

impl DrainWindow {
    pub fn new(now: Instant) -> Self {
        Self {
            delivered: 0,
            delivered_at: now,
            min_rtt: None,
            rates: VecDeque::new(),
        }
    }

    /// The bulk bytes allowed unacknowledged now.
    pub fn bytes(&self) -> usize {
        let (Some(rtt), Some(rate)) = (self.min_rtt, self.max_rate()) else {
            return FLOOR_BYTES;
        };
        ((rate * rtt.as_secs_f64() * GAIN).round() as usize).clamp(FLOOR_BYTES, CEILING_BYTES)
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
}
