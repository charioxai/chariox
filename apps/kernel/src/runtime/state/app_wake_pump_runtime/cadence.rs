//! Backlog pages drain immediately; a handler cannot re-arm the same wake
//! faster than the old one-second maintenance floor, even across revisions.
use crate::durable_state::app_wakes::AppWakeOperation;
use chariox_app_runtime::managed_state::{DueWake, MAX_WAKES};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const REDELIVERY_FLOOR: Duration = Duration::from_secs(1);
// One old maintenance page per second after the existing durable quota drains.
const CREDIT_PER_WAKE: Duration = Duration::from_millis(1_000 / super::PAGE as u64);
type Key = (String, String, String);
type Installation = (String, String);

#[derive(Default)]
pub(super) struct WakeCadence {
    recent: BTreeMap<Key, Instant>,
    buckets: BTreeMap<Installation, Bucket>,
}

struct Bucket {
    credit: Duration,
    observed: Instant,
}

fn ceiling() -> Duration {
    CREDIT_PER_WAKE * MAX_WAKES as u32
}

impl Bucket {
    fn refill(&mut self, now: Instant) {
        self.credit = (self.credit + now.duration_since(self.observed)).min(ceiling());
        self.observed = now;
    }
}

fn key(wake: &DueWake) -> Key {
    (
        wake.owner_id.clone(),
        wake.installation_id.clone(),
        wake.wake.id.clone(),
    )
}

impl WakeCadence {
    pub(super) fn record_attempt(&mut self, wake: &DueWake, now: Instant) {
        self.recent.insert(key(wake), now);
    }

    pub(super) fn split(
        &mut self,
        due: Vec<DueWake>,
        now_ms: u64,
        now: Instant,
    ) -> (Vec<DueWake>, Vec<AppWakeOperation>) {
        self.recent
            .retain(|_, last| now.duration_since(*last) < REDELIVERY_FLOOR);
        self.buckets.retain(|_, bucket| {
            bucket.refill(now);
            bucket.credit < ceiling()
        });
        let mut ready = Vec::new();
        let mut held = Vec::new();
        for wake in due {
            let same_id_wait = self
                .recent
                .get(&key(&wake))
                .map(|last| REDELIVERY_FLOOR - now.duration_since(*last));
            let bucket = self
                .buckets
                .entry((wake.owner_id.clone(), wake.installation_id.clone()))
                .or_insert(Bucket {
                    credit: ceiling(),
                    observed: now,
                });
            let remaining = same_id_wait
                .unwrap_or(Duration::ZERO)
                .max(CREDIT_PER_WAKE.saturating_sub(bucket.credit));
            if !remaining.is_zero() {
                held.push(AppWakeOperation::Postponed {
                    wake,
                    until_ms: now_ms.saturating_add(remaining.as_millis().max(1) as u64),
                });
            } else {
                bucket.credit -= CREDIT_PER_WAKE;
                ready.push(wake);
            }
        }
        (ready, held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_app_runtime::managed_state::Wake;

    fn due(id: &str, revision: &str) -> DueWake {
        DueWake {
            owner_id: "alice".into(),
            installation_id: "one".into(),
            wake: Wake {
                id: id.into(),
                revision: revision.into(),
                due_at_ms: 0,
            },
            attempts: 0,
            counts_as_use: false,
        }
    }

    #[test]
    fn rearmed_app_wake_waits_without_throttling_distinct_backlog() {
        let mut cadence = WakeCadence::default();
        let first = Instant::now();
        cadence.record_attempt(&due("recurring", "first"), first);
        for tick in 1..100 {
            let (ready, held) = cadence.split(
                vec![due("recurring", "new")],
                tick * 10,
                first + Duration::from_millis(tick * 10),
            );
            assert!(ready.is_empty());
            assert!(matches!(
                &held[..],
                [AppWakeOperation::Postponed { until_ms: 1000, .. }]
            ));
        }
        let (ready, held) = cadence.split(
            (0..120).map(|n| due(&n.to_string(), "backlog")).collect(),
            999,
            first + Duration::from_millis(999),
        );
        assert_eq!(ready.len(), 120);
        assert!(held.is_empty());
        let (ready, held) = cadence.split(
            vec![due("recurring", "new")],
            1000,
            first + REDELIVERY_FLOOR,
        );
        assert_eq!(ready.len(), 1);
        assert!(held.is_empty());
        assert!(cadence.recent.is_empty());
    }

    #[test]
    fn rotating_ids_cannot_bypass_installation_rate_after_full_quota_burst() {
        let mut cadence = WakeCadence::default();
        let first = Instant::now();
        let (ready, held) = cadence.split(
            (0..MAX_WAKES)
                .map(|n| due(&format!("old-{n}"), "backlog"))
                .collect(),
            0,
            first,
        );
        assert_eq!(ready.len(), MAX_WAKES);
        assert!(held.is_empty());
        let mut delivered = 0;
        for tick in 0..1_000 {
            let (ready, held) = cadence.split(
                vec![due(&format!("tick-{tick}"), "fresh")],
                tick,
                first + Duration::from_millis(tick),
            );
            delivered += ready.len();
            assert_eq!(ready.len() + held.len(), 1);
            assert!(delivered <= super::super::PAGE);
        }
        assert_eq!(delivered, super::super::PAGE - 1);
        let (ready, _) = cadence.split(
            vec![due("tick-1000", "fresh")],
            1_000,
            first + Duration::from_secs(1),
        );
        assert_eq!(ready.len(), 1);
        cadence.record_attempt(&due("recent", "first"), first + Duration::from_millis(1));
        let (ready, held) = cadence.split(
            vec![due("recent", "new")],
            1_000,
            first + Duration::from_secs(1),
        );
        assert!(ready.is_empty());
        assert!(matches!(
            &held[..],
            [AppWakeOperation::Postponed { until_ms: 1125, .. }]
        ));
        let mut independent = due("unrelated", "fresh");
        independent.installation_id = "two".into();
        let (ready, held) = cadence.split(vec![independent], 1_000, first + Duration::from_secs(1));
        assert_eq!(ready.len(), 1);
        assert!(held.is_empty());
        // A quiet installation regains its complete burst and needs no retained bucket.
        let (ready, held) = cadence.split(vec![], 40_000, first + Duration::from_secs(40));
        assert!(ready.is_empty() && held.is_empty());
        assert!(cadence.buckets.is_empty());
    }
}
