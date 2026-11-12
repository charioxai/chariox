//! How fast a command has grown over the last minute. A pause is sized by the
//! rate the command was growing at, and the last sample says little about that:
//! a build that took 300 MiB at once when it started measures a spike for one
//! sample and then grows slowly. The typical rate of the minute is what the
//! command needs room for.
use std::collections::VecDeque;

/// How far back the samples that decide the typical rate reach.
const SPAN_MS: u64 = 60_000;

#[derive(Default)]
pub(super) struct Window {
    /// Process-table time and growth per second of each sample, oldest first.
    samples: VecDeque<(u64, u64)>,
}

impl Window {
    pub(super) fn record(&mut self, at: u64, rate: u64) {
        self.samples.push_back((at, rate));
        while self
            .samples
            .front()
            .is_some_and(|(then, _)| at.saturating_sub(*then) > SPAN_MS)
        {
            self.samples.pop_front();
        }
    }

    /// The median rate of the window (the lower of the two middle ones),
    /// `None` before any sample.
    pub(super) fn typical(&self) -> Option<u64> {
        let mut rates = self
            .samples
            .iter()
            .map(|(_, rate)| *rate)
            .collect::<Vec<_>>();
        rates.sort_unstable();
        rates.get(rates.len().checked_sub(1)? / 2).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn window(rates: &[u64]) -> Window {
        let mut window = Window::default();
        for (at, rate) in rates.iter().enumerate() {
            window.record(at as u64 * 250, *rate);
        }
        window
    }

    #[test]
    fn a_startup_spike_barely_counts_against_the_steady_rate() {
        let window = window(&[0, 1200 * MIB, 0, 4 * MIB, 4 * MIB, 4 * MIB, 4 * MIB]);
        assert_eq!(window.typical(), Some(4 * MIB));
    }

    #[test]
    fn a_steady_fast_grower_reads_fast() {
        let window = window(&[0, 200 * MIB, 190 * MIB, 210 * MIB, 200 * MIB]);
        assert_eq!(window.typical(), Some(200 * MIB));
    }

    #[test]
    fn only_the_last_minute_counts() {
        let mut window = Window::default();
        for at in 0..400 {
            window.record(at * 150, 800 * MIB);
        }
        window.record(121_000, MIB);
        window.record(121_250, MIB);
        assert_eq!(window.typical(), Some(MIB));
    }

    #[test]
    fn an_empty_window_has_no_rate() {
        assert_eq!(Window::default().typical(), None);
    }
}
