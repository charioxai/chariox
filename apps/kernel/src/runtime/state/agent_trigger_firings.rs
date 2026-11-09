//! MP-08 / MP-09 / MP-10 / MP-11: the firing of a due trigger runs apart from the maintenance pump.
//! A firing can wait for as long as a provider lane stays held or a worker stays silent; it holds
//! only its own trigger, whose claim already keeps it from being fired twice.
use super::*;
use crate::session::AgentTriggerDispatch;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicUsize;

/// How many firings are admitted at once; a due trigger beyond that waits for a free place.
const FIRINGS_AT_ONCE: usize = 8;
/// How long a firing keeps its place. A firing that takes longer goes on to its end, because
/// stopping it half way would leave a steer hold behind, but it no longer holds back the others.
const PLACE_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(super) struct TriggerFirings {
    places: Arc<tokio::sync::Semaphore>,
    running: Arc<AtomicUsize>,
    idle: Arc<Notify>,
    queues: Arc<std::sync::Mutex<HashMap<String, VecDeque<(AgentTriggerDispatch, u64)>>>>,
}

impl Default for TriggerFirings {
    fn default() -> Self {
        Self {
            places: Arc::new(tokio::sync::Semaphore::new(FIRINGS_AT_ONCE)),
            running: Arc::default(),
            idle: Arc::default(),
            queues: Arc::default(),
        }
    }
}

/// The one task that fires the triggers of one agent, from the first claim to the last that is
/// waiting. Dropping it before the queue is empty (a panic) frees the agent for a new runner.
struct AgentRunner {
    agent_id: String,
    firings: TriggerFirings,
    _running: RunningFiring,
    drained: bool,
}

impl Drop for AgentRunner {
    fn drop(&mut self) {
        if !self.drained {
            self.firings.lock_queues().remove(&self.agent_id);
        }
    }
}

/// Counts one firing as running until it ends, however it ends.
struct RunningFiring {
    running: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Drop for RunningFiring {
    fn drop(&mut self) {
        if self.running.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_waiters();
        }
    }
}

impl TriggerFirings {
    fn lock_queues(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, VecDeque<(AgentTriggerDispatch, u64)>>> {
        self.queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Appends `dispatch` to the queue of its agent, which is shared by every pump. The agent
    /// gets a runner only when none is running, so its triggers never fire out of claim order.
    fn enqueue(&self, dispatch: AgentTriggerDispatch, now_ms: u64) -> Option<AgentRunner> {
        let agent_id = dispatch.agent_id.clone();
        let mut queues = self.lock_queues();
        if let Some(queue) = queues.get_mut(&agent_id) {
            queue.push_back((dispatch, now_ms));
            return None;
        }
        queues.insert(agent_id.clone(), VecDeque::from([(dispatch, now_ms)]));
        Some(AgentRunner {
            agent_id,
            firings: self.clone(),
            _running: self.begin(),
            drained: false,
        })
    }

    /// The next waiting firing of the runner's agent; the agent has no queue once none is left.
    fn next(&self, runner: &mut AgentRunner) -> Option<(AgentTriggerDispatch, u64)> {
        let mut queues = self.lock_queues();
        let next = queues.get_mut(&runner.agent_id)?.pop_front();
        if next.is_none() {
            queues.remove(&runner.agent_id);
            runner.drained = true;
        }
        next
    }

    fn begin(&self) -> RunningFiring {
        self.running.fetch_add(1, Ordering::AcqRel);
        RunningFiring {
            running: Arc::clone(&self.running),
            idle: Arc::clone(&self.idle),
        }
    }

    /// Every place taken, so that a claimed trigger waits to be fired.
    #[cfg(test)]
    pub(super) fn hold_all_places(&self) -> tokio::sync::OwnedSemaphorePermit {
        Arc::clone(&self.places)
            .try_acquire_many_owned(FIRINGS_AT_ONCE as u32)
            .expect("no firing runs yet")
    }

    #[cfg(test)]
    async fn settled(&self) {
        loop {
            let idle = self.idle.notified();
            tokio::pin!(idle);
            idle.as_mut().enable();
            if self.running.load(Ordering::Acquire) == 0 {
                return;
            }
            idle.await;
        }
    }
}

impl KernelRuntimeState {
    /// Fires the claimed `dispatches` as work of their own and settles each trigger with its
    /// result. The claim stays until then, so a trigger is neither fired again nor lost. The
    /// triggers of one agent fire one after another in the order they were claimed, across pumps
    /// too, because a steer needs its prompt at the head of that agent's queue; other agents do
    /// not wait for them.
    pub(super) fn spawn_trigger_firings(&self, dispatches: Vec<AgentTriggerDispatch>, now_ms: u64) {
        for dispatch in dispatches {
            let Some(mut runner) = self.trigger_firings.enqueue(dispatch, now_ms) else {
                continue;
            };
            let state = self.clone();
            tokio::spawn(async move {
                while let Some((dispatch, now_ms)) = state.trigger_firings.next(&mut runner) {
                    state.fire_and_settle_agent_trigger(&dispatch, now_ms).await;
                }
            });
        }
    }

    async fn fire_and_settle_agent_trigger(&self, dispatch: &AgentTriggerDispatch, now_ms: u64) {
        let place = Arc::clone(&self.trigger_firings.places)
            .acquire_owned()
            .await
            .expect("trigger firing places are never closed");
        let firing = self.fire_agent_trigger(dispatch);
        tokio::pin!(firing);
        let result = tokio::select! {
            result = &mut firing => {
                drop(place);
                result
            }
            () = tokio::time::sleep(PLACE_LIMIT) => {
                drop(place);
                tracing::warn!(trigger_id = %dispatch.trigger_id,
                    "MP-08/MP-09/MP-10/MP-11: a trigger firing is still waiting; it goes on without holding back other agents");
                firing.await
            }
        };
        self.settle_agent_trigger_firing(dispatch, result, now_ms);
    }

    /// Waits until no trigger is being fired.
    #[cfg(test)]
    pub(crate) async fn trigger_firings_settled(&self) {
        self.trigger_firings.settled().await;
    }

    /// Fires every due trigger and waits for the firings to end.
    #[cfg(test)]
    pub(crate) async fn fire_due_agent_triggers(&self, now_ms: u64) {
        self.dispatch_due_agent_triggers(now_ms);
        self.trigger_firings_settled().await;
    }
}
