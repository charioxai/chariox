//! MP-08 / MP-09 / MP-10 / MP-11: the slow part of supervising one task runs apart from the
//! lifecycle sweep. A worker, a peer or a provider that does not answer holds only the work of the
//! task it belongs to; the sweep goes on with every other agent and starts the work again when it
//! finds it undone.
use super::*;
use crate::durable_state::agent_lifecycle::AgentTaskExecution;
use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::time::Instant;

/// How long one piece of work may run. The longest wait inside it is a relay request to a worker
/// (`relay_request_timeout_ms`, 60 s by default), and a cancellation walks a task and its children.
const WORK_LIMIT: Duration = Duration::from_secs(180);
/// How long the sweep waits for one task's settlement before it moves on to the next task, and how
/// long it waits for all of them together in one pass. The work goes on apart from the sweep, up
/// to `WORK_LIMIT`, and the next sweep asks again. A pass stays well under the dead-man check
/// (three sweeps) even when several tasks hold.
pub(super) const TASK_WAIT: Duration = Duration::from_secs(5);
pub(super) const SWEEP_PATIENCE: Duration = Duration::from_secs(20);

/// How the work of one task ended as far as the sweep that asked for it is concerned.
pub(super) enum TaskStep<T> {
    Done(T),
    /// The same work still runs from an earlier sweep, or no place is left for it.
    Busy,
    /// The work did not end within the sweep's patience; it goes on apart from the sweep.
    TimedOut,
}

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
    /// Work a piece of running work starts itself, in a pool of its own (`claim_child`), with the
    /// same turns for the children turned away as the sweeps have in `waiting`.
    children: BTreeSet<String>,
    children_waiting: VecDeque<(String, Instant)>,
}

/// Admits `key` to `running` (at most `WORK_AT_ONCE` pieces) in the turn order of `waiting`.
/// False when the same work already runs or no place is left for it; the key then keeps, or
/// takes, its turn.
fn admit(
    running: &mut BTreeSet<String>,
    waiting: &mut VecDeque<(String, Instant)>,
    key: &str,
    now: Instant,
) -> bool {
    if running.contains(key) {
        return false;
    }
    waiting.retain(|(_, asked)| now.saturating_duration_since(*asked) <= TURN_LIMIT);
    let free = WORK_AT_ONCE.saturating_sub(running.len());
    let turn = waiting.iter().position(|(waiting, _)| waiting == key);
    let admitted = match turn {
        Some(turn) => turn < free,
        None => waiting.len() < free,
    };
    if !admitted {
        match turn {
            Some(turn) => waiting[turn].1 = now,
            None => waiting.push_back((key.to_owned(), now)),
        }
        return false;
    }
    if let Some(turn) = turn {
        waiting.remove(turn);
    }
    running.insert(key.to_owned());
    true
}

#[derive(Clone, Default)]
pub(super) struct TaskWorkFlight {
    admission: Arc<std::sync::Mutex<Admission>>,
    /// The last task a pass reached when it stopped for patience; the next pass starts after it.
    cursor: Arc<std::sync::Mutex<Option<String>>>,
}

/// `items` starting after the one `key` names `cursor`, then the ones before it: a pass that
/// stopped for patience goes on where it stopped, so the first tasks of the ledger do not spend
/// the patience of every pass. A cursor that names none of them starts from the first.
pub(super) fn after_cursor<T>(
    mut items: Vec<T>,
    cursor: Option<&str>,
    key: impl Fn(&T) -> &str,
) -> Vec<T> {
    if let Some(at) = cursor.and_then(|cursor| items.iter().position(|item| key(item) == cursor)) {
        items.rotate_left(at + 1);
    }
    items
}

/// The right to run the work named `key`, held until the work ends, however it ends.
pub(super) struct TaskWorkClaim {
    admission: Arc<std::sync::Mutex<Admission>>,
    key: String,
}

impl Drop for TaskWorkClaim {
    fn drop(&mut self) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        admission.running.remove(&self.key);
        admission.children.remove(&self.key);
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
        let Admission {
            running, waiting, ..
        } = &mut *admission;
        admit(running, waiting, key, now).then(|| TaskWorkClaim {
            admission: Arc::clone(&self.admission),
            key: key.to_owned(),
        })
    }

    /// The right to run the work that a piece of running work starts (the cancellation and the
    /// goal check a sweep of one task finds). Its own pool of `WORK_AT_ONCE` places and its own
    /// turns for the children turned away, apart from the places of the sweeps: a sweep that took
    /// the last place never parks its own cancellation behind itself, no place of the sweeps is
    /// reserved for a waiting child, and slow children left by sweeps that already ended stay
    /// bounded without the same ones taking every place again before a later one starts.
    fn claim_child(&self, key: &str) -> Option<TaskWorkClaim> {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Admission {
            running,
            children,
            children_waiting,
            ..
        } = &mut *admission;
        (!running.contains(key) && admit(children, children_waiting, key, Instant::now())).then(
            || TaskWorkClaim {
                admission: Arc::clone(&self.admission),
                key: key.to_owned(),
            },
        )
    }

    pub(super) fn after_cursor(&self, tasks: Vec<AgentTaskExecution>) -> Vec<AgentTaskExecution> {
        let cursor = self
            .cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        after_cursor(tasks, cursor.as_deref(), |task| task.task_id.as_str())
    }

    pub(super) fn set_cursor(&self, cursor: Option<String>) {
        *self
            .cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = cursor;
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        let admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        admission.running.is_empty() && admission.children.is_empty()
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
        let claim = self.task_work.claim(&key);
        self.run_task_work(key, claim, work);
    }

    /// `spawn_task_work` for work that a sweep's own work starts: it takes no place of its own.
    pub(super) fn spawn_child_task_work<Work, Done>(&self, key: String, work: Work)
    where
        Work: FnOnce(KernelRuntimeState) -> Done + Send + 'static,
        Done: Future<Output = Result<(), DaemonError>> + Send + 'static,
    {
        let claim = self.task_work.claim_child(&key);
        self.run_task_work(key, claim, work);
    }

    fn run_task_work<Work, Done>(&self, key: String, claim: Option<TaskWorkClaim>, work: Work)
    where
        Work: FnOnce(KernelRuntimeState) -> Done + Send + 'static,
        Done: Future<Output = Result<(), DaemonError>> + Send + 'static,
    {
        let Some(claim) = claim else {
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

    /// Runs the settlement of one task as its own work and waits for it at most `TASK_WAIT` and
    /// what is left of `patience`, so a task whose turn never settles holds only itself and
    /// never the pass of every other agent. The work keeps its claim until it ends, so the
    /// following sweeps pass it by instead of waiting again.
    pub(super) async fn sweep_task_within<Work, Done>(
        &self,
        key: String,
        patience: &mut Duration,
        work: Work,
    ) -> TaskStep<Result<Option<AgentTaskExecution>, DaemonError>>
    where
        Work: FnOnce(KernelRuntimeState) -> Done + Send + 'static,
        Done: Future<Output = Result<Option<AgentTaskExecution>, DaemonError>> + Send + 'static,
    {
        let Some(claim) = self.task_work.claim(&key) else {
            return TaskStep::Busy;
        };
        let (answer, answered) = tokio::sync::oneshot::channel();
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
            match tokio::time::timeout(WORK_LIMIT, work(state.clone())).await {
                Ok(settled) => {
                    if let Err(settled) = answer.send(settled) {
                        // The sweep moved on: the work reports what it ended itself.
                        match settled {
                            Ok(ended) => state.record_task_transition_notices(ended),
                            Err(error) => {
                                tracing::warn!(%key, error=%crate::secret_redaction::redact_secrets(&error.to_string()),
                                "MP-08/MP-09/MP-10/MP-11: task settlement retained for the next sweep")
                            }
                        }
                    }
                }
                Err(_) => {
                    tracing::warn!(%key, "MP-08/MP-09/MP-10/MP-11: task settlement ran out of time; the next sweep starts it again");
                }
            }
        });
        let wait = TASK_WAIT.min(*patience);
        let started = Instant::now();
        let outcome = tokio::time::timeout(wait, answered).await;
        *patience = patience.saturating_sub(started.elapsed());
        match outcome {
            Ok(Ok(settled)) => TaskStep::Done(settled),
            Ok(Err(_)) => TaskStep::Busy,
            Err(_) => TaskStep::TimedOut,
        }
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
    fn a_pass_that_stopped_goes_on_after_the_last_task_it_reached() {
        let items = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        let order = |cursor| after_cursor(items.clone(), cursor, |item| item.as_str());
        assert_eq!(order(None), ["a", "b", "c", "d", "e"]);
        assert_eq!(order(Some("b")), ["c", "d", "e", "a", "b"]);
        assert_eq!(order(Some("e")), ["a", "b", "c", "d", "e"]);
        assert_eq!(
            order(Some("gone")),
            ["a", "b", "c", "d", "e"],
            "a task that left the set restarts from the first"
        );
    }

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
    fn a_sweeps_own_work_starts_when_the_sweep_took_the_last_place() {
        let flight = TaskWorkFlight::default();
        let mut busy = (0..WORK_AT_ONCE - 1)
            .map(|i| flight.claim(&format!("inbox:room:agent-{i}")).unwrap())
            .collect::<Vec<_>>();
        // The sweep of the cancelled task takes the last place...
        let sweep = flight
            .claim("sweep-task:task-1")
            .expect("one place is left");
        // ...and the cancellation it finds is still started, not parked behind the sweep.
        let cancel = flight.claim_child("cancel:task-1");
        assert!(
            cancel.is_some(),
            "the cancellation of a task whose sweep holds the last place never started"
        );
        drop(sweep);
        drop(cancel);
        // The place the sweep gave up is not kept back for work that never waited for one.
        busy.truncate(WORK_AT_ONCE - 2);
        assert!(
            flight.claim("sweep-task:task-2").is_some(),
            "a free place stays free for the next sweep"
        );
        assert!(flight.claim("sweep-task:task-1").is_some());
    }

    #[test]
    fn work_a_sweep_starts_is_bounded_and_still_makes_progress_at_saturation() {
        let flight = TaskWorkFlight::default();
        // Every place of the sweeps is taken, and more sweeps than places each leave a slow
        // child behind after their own claim ended (a goal check starts its child and returns).
        let sweeps = (0..WORK_AT_ONCE)
            .map(|i| flight.claim(&format!("sweep-task:{i}")).unwrap())
            .collect::<Vec<_>>();
        let mut children = (0..WORK_AT_ONCE + 8)
            .filter_map(|i| flight.claim_child(&format!("goal-check:{i}")))
            .collect::<Vec<_>>();
        assert_eq!(
            children.len(),
            WORK_AT_ONCE,
            "slow children are bounded however many sweeps left one behind"
        );
        drop(sweeps);
        // A child that ended frees a place for the next one; the sweeps' own places are not held.
        children.pop();
        // The child turned away first (the 33rd) takes it; a newcomer waits behind the others.
        let first_turned_away = flight.claim_child(&format!("goal-check:{WORK_AT_ONCE}"));
        assert!(first_turned_away.is_some());
        assert!(flight.claim_child("goal-check:later").is_none());
        assert!(flight.claim("sweep-task:next").is_some());
    }

    #[test]
    fn a_child_turned_away_gets_a_place_before_the_children_that_filled_them() {
        let flight = TaskWorkFlight::default();
        let slow = |i: usize| format!("cancel:slow-{i:02}");
        let held = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_child(&slow(i)).unwrap())
            .collect::<Vec<_>>();
        // A later cancellation is turned away: every child place is taken.
        assert!(flight.claim_child("cancel:later").is_none());
        // Waiting children never reserve a place of the sweeps.
        assert!(flight.claim("sweep-task:any").is_some());
        drop(held);

        // The slow cancellations fail again and are asked for first, as the enumeration in task
        // order does; the last place is kept for the child that was turned away.
        let mut again = Vec::new();
        for i in 0..WORK_AT_ONCE {
            again.push(flight.claim_child(&slow(i)));
        }
        assert!(
            again.last().unwrap().is_none(),
            "the last child place is kept for the child that was turned away"
        );
        assert!(
            flight.claim_child("cancel:later").is_some(),
            "the child that was turned away starts before the original 32 take every place again"
        );
    }

    #[test]
    fn children_turned_away_are_served_oldest_first() {
        let flight = TaskWorkFlight::default();
        let mut held = (0..WORK_AT_ONCE)
            .map(|i| flight.claim_child(&format!("busy-{i}")).unwrap())
            .collect::<Vec<_>>();
        for key in ["first", "second", "third"] {
            assert!(flight.claim_child(key).is_none());
        }
        held.truncate(WORK_AT_ONCE - 2);
        assert!(flight.claim_child("newcomer").is_none());
        let first = flight.claim_child("first");
        let second = flight.claim_child("second");
        assert!(first.is_some() && second.is_some());
        assert!(flight.claim_child("third").is_none(), "no place is left");
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
