//! The renewal of the machine list that `ListRemoteMachines` requests wait for. One pass at a
//! time runs as its own task, so a request that stops waiting never cancels it; requests share it.
//!
//! States and transitions. Each transition is one compare-and-set under the lock: a request
//! decides and moves the state in one step, and a pass settles only the generation it started.
//!
//! | state | event | next | test |
//! |---|---|---|---|
//! | Idle | request | Renewing(gen 1, started now) | a_request_starts_a_renewal_and_waits_for_its_list |
//! | Renewing started at or after the request's bound | request | Renewing (the request joins it) | a_request_joins_a_renewal_that_started_within_its_bound |
//! | Renewing(gen) | its pass succeeds | Fresh(gen) | a_request_starts_a_renewal_and_waits_for_its_list |
//! | Renewing(gen) | its pass fails | Failed(gen, reason) | a_failed_renewal_is_a_failure_with_its_reason |
//! | Renewing(gen), superseded by gen+1 | its pass ends | unchanged | a_superseded_renewal_does_not_settle_the_newer_one |
//! | Fresh or Failed started at or after the bound | request | unchanged (the request gets that result) | a_renewal_that_finishes_as_a_second_request_arrives_answers_both |
//! | Fresh, Failed or Renewing started before the bound | request | Renewing(gen+1) | a_finished_renewal_that_went_stale_is_not_reused |
//!
//! A superseded renewal keeps running; this table settles only who waits for which renewal. What
//! a pass writes is fenced where it writes: the projection numbers every pass when it starts and
//! only the newest to start publishes (`RemoteRelayInventoryProjectionStore::publish_live`).
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

/// How a renewal ended: Ok, or the reason it failed.
pub(crate) type RenewalOutcome = Result<(), String>;

#[derive(Debug)]
enum RenewalState {
    Idle,
    Renewing {
        generation: u64,
        started_at_ms: u64,
        outcome: watch::Receiver<Option<RenewalOutcome>>,
    },
    Settled {
        started_at_ms: u64,
        outcome: RenewalOutcome,
    },
}

struct Renewals {
    state: RenewalState,
    generations: u64,
}

/// What a request gets: the result of a renewal that already settled within its bound, or the
/// renewal it waits for.
pub(crate) enum RenewalWait {
    Settled(RenewalOutcome),
    Running(watch::Receiver<Option<RenewalOutcome>>),
}

#[derive(Clone)]
pub(crate) struct MachineListRenewal(Arc<Mutex<Renewals>>);

impl Default for MachineListRenewal {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Renewals {
            state: RenewalState::Idle,
            generations: 0,
        })))
    }
}

impl MachineListRenewal {
    /// A request whose list must be no older than `bound_ms`: it gets a renewal that started at or
    /// after `bound_ms` (running or settled); otherwise it starts one at `now_ms` with `start`.
    pub(crate) fn request<F>(
        &self,
        bound_ms: u64,
        now_ms: u64,
        start: impl FnOnce() -> F + Send + 'static,
    ) -> RenewalWait
    where
        F: Future<Output = RenewalOutcome> + Send + 'static,
    {
        let mut renewals = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &renewals.state {
            RenewalState::Renewing {
                started_at_ms,
                outcome,
                ..
            } if *started_at_ms >= bound_ms => return RenewalWait::Running(outcome.clone()),
            RenewalState::Settled {
                started_at_ms,
                outcome,
                ..
            } if *started_at_ms >= bound_ms => return RenewalWait::Settled(outcome.clone()),
            _ => {}
        }
        renewals.generations += 1;
        let generation = renewals.generations;
        let (sender, receiver) = watch::channel(None);
        renewals.state = RenewalState::Renewing {
            generation,
            started_at_ms: now_ms,
            outcome: receiver.clone(),
        };
        drop(renewals);
        let renewal = self.clone();
        tokio::spawn(async move {
            // The pass runs as a task of its own: a panic ends it with a reason, not a stuck state.
            let outcome = match tokio::spawn(async move { start().await }).await {
                Ok(outcome) => outcome,
                Err(error) => Err(format!("the machine list renewal stopped: {error}")),
            };
            renewal.settle(generation, outcome.clone());
            let _ = sender.send(Some(outcome));
        });
        RenewalWait::Running(receiver)
    }

    /// Renewing(generation) -> Fresh / Failed; a renewal another one superseded settles nothing.
    fn settle(&self, generation: u64, outcome: RenewalOutcome) {
        let mut renewals = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let RenewalState::Renewing {
            generation: running,
            started_at_ms,
            ..
        } = renewals.state
        {
            if running == generation {
                renewals.state = RenewalState::Settled {
                    started_at_ms,
                    outcome,
                };
            }
        }
    }

    #[cfg(test)]
    fn settled(&self) -> Option<RenewalOutcome> {
        match &self.0.lock().unwrap().state {
            RenewalState::Settled { outcome, .. } => Some(outcome.clone()),
            _ => None,
        }
    }
}

/// Waits at most `budget` for what `wait` stands for: Some(outcome) once the renewal settled,
/// None while it still runs.
pub(crate) async fn outcome_within(
    wait: RenewalWait,
    budget: std::time::Duration,
) -> Option<RenewalOutcome> {
    match wait {
        RenewalWait::Settled(outcome) => Some(outcome),
        RenewalWait::Running(mut receiver) => {
            match tokio::time::timeout(budget, receiver.wait_for(Option::is_some)).await {
                Err(_) => None,
                Ok(Ok(outcome)) => outcome.clone(),
                Ok(Err(_)) => Some(Err(
                    "the machine list renewal ended without an outcome".to_string()
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::oneshot;

    /// A pass the test ends by hand, counting how many passes started.
    fn pass(
        started: &Arc<AtomicUsize>,
    ) -> (
        impl FnOnce() -> futures_util::future::BoxFuture<'static, RenewalOutcome> + Send + 'static,
        oneshot::Sender<RenewalOutcome>,
    ) {
        use futures_util::FutureExt;
        let (end, ended) = oneshot::channel::<RenewalOutcome>();
        let started = Arc::clone(started);
        (
            move || {
                started.fetch_add(1, Ordering::SeqCst);
                async move { ended.await.unwrap_or_else(|_| Err("dropped".into())) }.boxed()
            },
            end,
        )
    }

    const LONG: Duration = Duration::from_secs(5);
    const SHORT: Duration = Duration::from_millis(30);

    #[tokio::test]
    async fn a_request_starts_a_renewal_and_waits_for_its_list() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (start, end) = pass(&started);
        let wait = renewal.request(0, 100, start);
        end.send(Ok(())).unwrap();
        assert_eq!(outcome_within(wait, LONG).await, Some(Ok(())));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(renewal.settled(), Some(Ok(())));
    }

    #[tokio::test]
    async fn a_request_joins_a_renewal_that_started_within_its_bound() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (start, end) = pass(&started);
        let first = renewal.request(0, 100, start);
        let (unused, _) = pass(&started);
        let second = renewal.request(90, 120, unused);
        assert_eq!(outcome_within(first, SHORT).await, None, "still running");
        end.send(Ok(())).unwrap();
        assert_eq!(outcome_within(second, LONG).await, Some(Ok(())));
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "the second request joined"
        );
    }

    // #1474 review 5969448770 P2-2: a failed renewal is a failure, with its reason, never a
    // successful answer of the stale list.
    #[tokio::test]
    async fn a_failed_renewal_is_a_failure_with_its_reason() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (start, end) = pass(&started);
        let wait = renewal.request(0, 100, start);
        end.send(Err("relay metadata query timed out".into()))
            .unwrap();
        assert_eq!(
            outcome_within(wait, LONG).await,
            Some(Err("relay metadata query timed out".into()))
        );
        let (unused, _) = pass(&started);
        assert!(
            matches!(renewal.request(50, 130, unused), RenewalWait::Settled(Err(reason)) if reason == "relay metadata query timed out"),
            "a request within the failed pass's bound gets the failure"
        );
    }

    // #1474 review 5969448770 P2-1: a pass whose last waiter gave up finishes on its own; once
    // its list is older than a new request's bound, that request starts a new pass.
    #[tokio::test]
    async fn a_finished_renewal_that_went_stale_is_not_reused() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (start, end) = pass(&started);
        let wait = renewal.request(0, 100, start);
        assert_eq!(
            outcome_within(wait, SHORT).await,
            None,
            "the waiter gave up"
        );
        end.send(Ok(())).unwrap();
        tokio::time::timeout(LONG, async {
            while renewal.settled().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the pass settles with no one waiting");
        // Another stale interval later: the list from the pass that started at 100 is too old.
        let (start, end) = pass(&started);
        let wait = renewal.request(2_100, 3_100, start);
        end.send(Ok(())).unwrap();
        assert_eq!(outcome_within(wait, LONG).await, Some(Ok(())));
        assert_eq!(started.load(Ordering::SeqCst), 2, "a new pass started");
        assert_eq!(renewal.settled(), Some(Ok(())));
    }

    #[tokio::test]
    async fn a_renewal_that_finishes_as_a_second_request_arrives_answers_both() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (start, end) = pass(&started);
        let first = renewal.request(0, 100, start);
        end.send(Ok(())).unwrap();
        // The second request arrives while the pass is settling, or just after.
        let (unused, _) = pass(&started);
        let second = renewal.request(90, 110, unused);
        assert_eq!(outcome_within(second, LONG).await, Some(Ok(())));
        assert_eq!(outcome_within(first, LONG).await, Some(Ok(())));
        assert_eq!(started.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_superseded_renewal_does_not_settle_the_newer_one() {
        let renewal = MachineListRenewal::default();
        let started = Arc::new(AtomicUsize::new(0));
        let (old_start, old_end) = pass(&started);
        let old = renewal.request(0, 100, old_start);
        let (new_start, new_end) = pass(&started);
        let new = renewal.request(2_000, 3_000, new_start);
        old_end.send(Err("old relay".into())).unwrap();
        assert_eq!(
            outcome_within(old, LONG).await,
            Some(Err("old relay".into()))
        );
        assert_eq!(renewal.settled(), None, "the newer pass is still renewing");
        new_end.send(Ok(())).unwrap();
        assert_eq!(outcome_within(new, LONG).await, Some(Ok(())));
        assert_eq!(renewal.settled(), Some(Ok(())));
    }
}
