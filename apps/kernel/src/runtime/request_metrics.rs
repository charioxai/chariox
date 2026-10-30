//! Bounded, payload-free timings for completed requests.
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const WINDOW: usize = 128;
const MAX_KINDS: usize = 1_024;

tokio::task_local! {
    static CURRENT: RequestObservation;
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RequestMetrics(Arc<Mutex<BTreeMap<String, VecDeque<RequestObservation>>>>);

#[derive(Debug, Clone, Default)]
struct Sample {
    queue_us: u64,
    handler_us: u64,
    durable_wait_us: u64,
    durable_commit_us: u64,
    fsync_us: u64,
    response_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestObservation(Arc<Mutex<Sample>>);

impl PartialEq for RequestObservation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RequestObservation {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingDistribution {
    pub p50: u64,
    pub p95: u64,
    pub max: u64,
}

impl TimingDistribution {
    fn from_values(values: impl Iterator<Item = u64>) -> Self {
        let mut values: Vec<_> = values.collect();
        values.sort_unstable();
        if values.is_empty() {
            return Self::default();
        }
        let percentile =
            |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
        Self {
            p50: percentile(50),
            p95: percentile(95),
            max: *values.last().unwrap(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestKindTiming {
    pub kind: String,
    pub samples: usize,
    pub queue_us: TimingDistribution,
    pub handler_us: TimingDistribution,
    pub durable_wait_us: TimingDistribution,
    pub durable_commit_us: TimingDistribution,
    pub fsync_us: TimingDistribution,
    pub response_bytes: TimingDistribution,
}

impl RequestObservation {
    pub(crate) fn current() -> Option<Self> {
        CURRENT.try_with(Clone::clone).ok()
    }

    pub(crate) async fn scope<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(self.clone(), future).await
    }

    pub(crate) fn record_queue(&self, duration: Duration) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).queue_us += micros(duration);
    }

    pub(crate) fn record_durable_wait(&self, duration: Duration) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .durable_wait_us += micros(duration);
    }

    pub(crate) fn record_fsync(&self, micros: u64) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).fsync_us += micros;
    }

    pub(crate) fn record_durable_commit(&self, duration: Duration) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .durable_commit_us += micros(duration);
    }
}

impl RequestMetrics {
    pub(crate) async fn observe<F>(&self, kind: String, future: F) -> F::Output
    where
        F: Future<Output = Result<crate::local::LocalDaemonResponse, crate::error::DaemonError>>,
    {
        let observation = RequestObservation(Arc::new(Mutex::new(Sample::default())));
        let started = Instant::now();
        let result = observation.scope(future).await;
        {
            let mut sample = observation.0.lock().unwrap_or_else(|e| e.into_inner());
            sample.handler_us = micros(started.elapsed()).saturating_sub(sample.queue_us);
        }
        let mut counter = ByteCounter::default();
        if let Ok(response) = &result {
            if serde_json::to_writer(&mut counter, response).is_ok() {
                observation
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .response_bytes = counter.0;
            }
        }
        self.record_observation(kind, observation);
        result
    }

    #[cfg(test)]
    fn record(&self, kind: String, sample: Sample) {
        self.record_observation(kind, RequestObservation(Arc::new(Mutex::new(sample))));
    }

    fn record_observation(&self, kind: String, sample: RequestObservation) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Command kinds come from the shared request enum, never caller text.
        if !state.contains_key(&kind) && state.len() == MAX_KINDS {
            return;
        }
        let samples = state.entry(kind).or_default();
        if samples.len() == WINDOW {
            samples.pop_front();
        }
        samples.push_back(sample);
    }

    pub(crate) fn snapshot(&self) -> Vec<RequestKindTiming> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        state
            .into_iter()
            .map(|(kind, observations)| {
                let samples: Vec<_> = observations
                    .iter()
                    .map(|observation| {
                        observation
                            .0
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone()
                    })
                    .collect();
                RequestKindTiming {
                    kind,
                    samples: samples.len(),
                    queue_us: TimingDistribution::from_values(samples.iter().map(|s| s.queue_us)),
                    handler_us: TimingDistribution::from_values(
                        samples.iter().map(|s| s.handler_us),
                    ),
                    durable_wait_us: TimingDistribution::from_values(
                        samples.iter().map(|s| s.durable_wait_us),
                    ),
                    durable_commit_us: TimingDistribution::from_values(
                        samples.iter().map(|s| s.durable_commit_us),
                    ),
                    fsync_us: TimingDistribution::from_values(samples.iter().map(|s| s.fsync_us)),
                    response_bytes: TimingDistribution::from_values(
                        samples.iter().map(|s| s.response_bytes),
                    ),
                }
            })
            .collect()
    }
}

fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

#[derive(Default)]
struct ByteCounter(u64);
impl std::io::Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buffer.len() as u64);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_window_drops_old_stalls_and_bounds_cardinality() {
        let metrics = RequestMetrics::default();
        metrics.record(
            "session.create".into(),
            Sample {
                handler_us: 2_000_000,
                ..Sample::default()
            },
        );
        for _ in 0..WINDOW {
            metrics.record(
                "session.create".into(),
                Sample {
                    handler_us: 10,
                    ..Sample::default()
                },
            );
        }
        let sample = &metrics.snapshot()[0];
        assert_eq!(sample.samples, WINDOW);
        assert_eq!(sample.handler_us.max, 10);
        for n in 0..MAX_KINDS * 2 {
            metrics.record(format!("kind.{n}"), Sample::default());
        }
        assert_eq!(metrics.snapshot().len(), MAX_KINDS);
    }

    #[tokio::test]
    async fn concurrent_requests_keep_their_own_durable_wait() {
        let slow = RequestObservation(Arc::new(Mutex::new(Sample::default())));
        let fast = RequestObservation(Arc::new(Mutex::new(Sample::default())));
        let slow_future = slow.scope(async {
            tokio::task::yield_now().await;
            RequestObservation::current()
                .unwrap()
                .record_durable_wait(Duration::from_secs(2));
            1u32
        });
        let fast_future = fast.scope(async { 2u32 });
        assert_eq!(tokio::join!(slow_future, fast_future), (1, 2));
        assert_eq!(fast.0.lock().unwrap().durable_wait_us, 0);
        assert_eq!(slow.0.lock().unwrap().durable_wait_us, 2_000_000);
    }
}
