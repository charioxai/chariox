//! MP-08 / MP-09 / MP-10 / MP-11: the firing of a due trigger runs apart from the maintenance pump.
//! A firing can wait for as long as a provider lane stays held or a worker stays silent; it holds
//! only its own trigger, whose claim already keeps it from being fired twice.
use super::*;
use crate::session::AgentTriggerDispatch;
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
}

impl Default for TriggerFirings {
    fn default() -> Self {
        Self {
            places: Arc::new(tokio::sync::Semaphore::new(FIRINGS_AT_ONCE)),
            running: Arc::default(),
            idle: Arc::default(),
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
    /// triggers of one agent fire one after another in the order they were claimed, because a
    /// steer needs its prompt at the head of that agent's queue; other agents do not wait for them.
    pub(super) fn spawn_trigger_firings(&self, dispatches: Vec<AgentTriggerDispatch>, now_ms: u64) {
        let mut by_agent: Vec<Vec<AgentTriggerDispatch>> = Vec::new();
        for dispatch in dispatches {
            match by_agent
                .iter_mut()
                .find(|group| group[0].agent_id == dispatch.agent_id)
            {
                Some(group) => group.push(dispatch),
                None => by_agent.push(vec![dispatch]),
            }
        }
        for group in by_agent {
            let running = self.trigger_firings.begin();
            let state = self.clone();
            tokio::spawn(async move {
                let _running = running;
                for dispatch in group {
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
