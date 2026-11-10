//! MP-08 / MP-09 / MP-10 / MP-11: the slow part of supervising one task runs apart from the
//! lifecycle sweep. A worker, a peer or a provider that does not answer holds only the work of the
//! task it belongs to; the sweep goes on with every other agent and starts the work again when it
//! finds it undone.
use super::*;
use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::time::Instant;

/// How long one piece of work may run. The longest wait inside it is a relay request to a worker
/// (`relay_request_timeout_ms`, 60 s by default), and a cancellation walks a task and its children.
const WORK_LIMIT: Duration = Duration::from_secs(180);
/// How many pieces of work run at once; the sweep that finds more undone starts them later.
pub(super) const WORK_AT_ONCE: usize = 32;

/// How long work that was turned away for lack of a place keeps its turn without being asked for
/// again. The sweeps ask every few seconds, so work not asked for within this time is no longer
/// undone and must not hold a place back from the others.
const TURN_LIMIT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Admission {
    running: BTreeSet<String>,
    /// Work turned away for lack of a place, oldest first, with when it was last asked for.
    waiting: VecDeque<(String, Instant)>,
}

#[derive(Clone, Default)]
pub(super) struct TaskWorkFlight {
    admission: Arc<std::sync::Mutex<Admission>>,
}

/// The right to run the work named `key`, held until the work ends, however it ends.
pub(super) struct TaskWorkClaim {
    admission: Arc<std::sync::Mutex<Admission>>,
    key: String,
}

impl Drop for TaskWorkClaim {
    fn drop(&mut self) {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .running
            .remove(&self.key);
    }
}

impl TaskWorkFlight {
    /// `None` when the same work already runs or no place is left for it. Work turned away for
    /// lack of a place keeps its turn: the places that free up go to it, oldest first, before
    /// they go to work that was not turned away, so the same enumeration of undone work cannot
    /// keep the first `WORK_AT_ONCE` pieces in the places for ever.
    fn claim(&self, key: &str) -> Option<TaskWorkClaim> {
        self.claim_at(key, Instant::now())
    }

    fn claim_at(&self, key: &str, now: Instant) -> Option<TaskWorkClaim> {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if admission.running.contains(key) {
            return None;
        }
        admission
            .waiting
            .retain(|(_, asked)| now.saturating_duration_since(*asked) <= TURN_LIMIT);
        let free = WORK_AT_ONCE.saturating_sub(admission.running.len());
        let turn = admission
            .waiting
            .iter()
            .position(|(waiting, _)| waiting == key);
        let admitted = match turn {
            Some(turn) => turn < free,
            None => admission.waiting.len() < free,
        };
        if !admitted {
            match turn {
                Some(turn) => admission.waiting[turn].1 = now,
                None => admission.waiting.push_back((key.to_owned(), now)),
            }
            return None;
        }
        if let Some(turn) = turn {
            admission.waiting.remove(turn);
        }
        admission.running.insert(key.to_owned());
        Some(TaskWorkClaim {
            admission: Arc::clone(&self.admission),
            key: key.to_owned(),
        })
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .running
            .is_empty()
    }
}

impl KernelRuntimeState {
    /// Runs `work` for the task named by `key` off the caller's path, at most once at a time and
    /// for at most `WORK_LIMIT`. Its failure stays supervised: the next sweep starts it again.
    pub(super) fn spawn_task_work<Work, Done>(&self, key: String, work: Work)
    where
        Work: FnOnce(KernelRuntimeState) -> Done + Send + 'static,
        Done: Future<Output = Result<(), DaemonError>> + Send + 'static,
    {
        let Some(claim) = self.task_work.claim(&key) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
            match tokio::time::timeout(WORK_LIMIT, work(state)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%key, error=%crate::secret_redaction::redact_secrets(&error.to_string()),
                        "MP-08/MP-09/MP-10/MP-11: task work retained for the next sweep");
                }
                Err(_) => {
                    tracing::warn!(%key, "MP-08/MP-09/MP-10/MP-11: task work ran out of time; the next sweep starts it again");
                }
            }
        });
    }

    /// Waits until no task work runs.
    #[cfg(test)]
    pub(crate) async fn task_work_settled(&self) {
        while !self.task_work.is_idle() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_work_does_not_start_twice_while_it_runs() {
        let flight = TaskWorkFlight::default();
        let first = flight.claim("cancel:task-1");
        assert!(first.is_some());
        assert!(flight.claim("cancel:task-1").is_none());
        assert!(
            flight.claim("goal-check:task-1").is_some(),
            "another kind of work, or another task, is not held by it"
        );
        drop(first);
        assert!(
            flight.claim("cancel:task-1").is_some(),
            "work that ended, however it ended, starts again"
        );
    }

    #[test]
    fn work_beyond_the_limit_waits_for_a_later_sweep() {
        let flight = TaskWorkFlight::default();
        let running = (0..WORK_AT_ONCE)
            .map(|i| flight.claim(&format!("cancel:task-{i}")).unwrap())
            .collect::<Vec<_>>();
        assert!(flight.claim("cancel:one-more").is_none());
        drop(running);
        assert!(flight.is_idle());
        assert!(flight.claim("cancel:one-more").is_some());
    }

    #[test]
    fn work_turned_away_gets_a_place_before_the_work_that_filled_them() {
        let flight = TaskWorkFlight::default();
        let now = Instant::now();
        let blocked = |i: usize| format!("provider-launch-retry:blocked-{i:02}");
        let running = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_at(&blocked(i), now).unwrap())
            .collect::<Vec<_>>();
        assert!(flight
            .claim_at("provider-launch-retry:ready", now)
            .is_none());
        drop(running);

        // The blocked work is asked for first again, as an enumeration in key order does.
        let mut again = Vec::new();
        for i in 0..WORK_AT_ONCE {
            again.push(flight.claim_at(&blocked(i), now));
        }
        assert!(
            again.last().unwrap().is_none(),
            "the last place is kept for the work that was turned away"
        );
        assert!(flight
            .claim_at("provider-launch-retry:ready", now)
            .is_some());
    }

    #[test]
    fn work_turned_away_is_served_oldest_first() {
        let flight = TaskWorkFlight::default();
        let now = Instant::now();
        let mut running = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_at(&format!("busy-{i}"), now).unwrap())
            .collect::<Vec<_>>();
        for key in ["first", "second", "third"] {
            assert!(flight.claim_at(key, now).is_none());
        }
        running.truncate(WORK_AT_ONCE - 2);
        assert!(flight.claim_at("third", now).is_none());
        assert!(flight.claim_at("newcomer", now).is_none());
        let first = flight.claim_at("first", now);
        let second = flight.claim_at("second", now);
        assert!(first.is_some() && second.is_some());
        assert!(flight.claim_at("third", now).is_none(), "no place is left");
    }

    #[test]
    fn work_that_is_no_longer_asked_for_does_not_keep_a_place_back() {
        let flight = TaskWorkFlight::default();
        let now = Instant::now();
        let running = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_at(&format!("busy-{i}"), now).unwrap())
            .collect::<Vec<_>>();
        assert!(flight.claim_at("gone", now).is_none());
        drop(running);
        let later = now + TURN_LIMIT + Duration::from_secs(1);
        let all = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_at(&format!("busy-{i}"), later))
            .collect::<Vec<_>>();
        assert!(
            all.iter().all(Option::is_some),
            "a turn that was not asked for again expires"
        );
    }
}
