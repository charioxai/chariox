//! Backlog pages drain immediately; a handler cannot re-arm the same wake
//! faster than the old one-second maintenance floor, even across revisions.
use crate::durable_state::app_wakes::AppWakeOperation;
use chariox_app_runtime::managed_state::DueWake;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const REDELIVERY_FLOOR: Duration = Duration::from_secs(1);
type Key = (String, String, String);

#[derive(Default)]
pub(super) struct WakeCadence(BTreeMap<Key, Instant>);

fn key(wake: &DueWake) -> Key {
    (
        wake.owner_id.clone(),
        wake.installation_id.clone(),
        wake.wake.id.clone(),
    )
}

impl WakeCadence {
    pub(super) fn record_attempt(&mut self, wake: &DueWake, now: Instant) {
        self.0.insert(key(wake), now);
    }

    pub(super) fn split(
        &mut self,
        due: Vec<DueWake>,
        now_ms: u64,
        now: Instant,
    ) -> (Vec<DueWake>, Vec<AppWakeOperation>) {
        self.0
            .retain(|_, last| now.duration_since(*last) < REDELIVERY_FLOOR);
        let mut ready = Vec::new();
        let mut held = Vec::new();
        for wake in due {
            if let Some(last) = self.0.get(&key(&wake)) {
                let remaining = REDELIVERY_FLOOR - now.duration_since(*last);
                held.push(AppWakeOperation::Postponed {
                    wake,
                    until_ms: now_ms.saturating_add(remaining.as_millis().max(1) as u64),
                });
            } else {
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
        assert!(cadence.0.is_empty());
    }
}
