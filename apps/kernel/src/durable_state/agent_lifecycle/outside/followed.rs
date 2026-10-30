//! MP-08 / MP-09 / MP-10 / MP-11: what the subscriptions follow, and what they were last given.
//!
//! Every session update asks whether an outside subscription cares. Decoding the subscriptions
//! for that answer, and handing the single durable writer every followed run state whether it
//! changed or not, starved every other durable write. The answer is cached here; the writer
//! drops it after a commit that changes the followed targets and after any outside mutation
//! that did not commit, so the next publication re-reads the targets and re-offers every state.
use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub(crate) struct Followed {
    /// Bumped by the writer; a cache from an older generation is reloaded.
    generation: AtomicU64,
    cache: Mutex<Cache>,
}

type Published = (crate::local::EventRunState, Option<String>);

fn published(state: &RunState) -> Published {
    (
        state.state,
        state.turn.as_ref().map(|turn| turn.prompt_id.clone()),
    )
}

#[derive(Debug, Default)]
struct Cache {
    /// `None` until the targets were first read.
    generation: Option<u64>,
    targets: Vec<EventTarget>,
    /// Last run state and settled turn queued for the writer per (session, agent). A turn
    /// that ends into a queued one keeps the agent working, so the turn is part of the key.
    published: BTreeMap<(String, String), Published>,
}

impl Followed {
    pub(in crate::durable_state) fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }
}

/// Subscriptions drop their targets when the session or agent is gone.
pub(super) fn removes_target(event: &OutsideEvent) -> bool {
    matches!(
        event.value,
        OutsideEventValue::Session {
            state: crate::local::EventSessionState::Ended
                | crate::local::EventSessionState::AgentRemoved
        }
    )
}

impl DurableKernelStateStore {
    /// Queues the run states of `session` (of every followed session when `None`) that differ
    /// from what the writer was last given, and returns whether anything was queued. `capture`
    /// reads the live state of the followed targets; `None` means it cannot read them now and
    /// the next publication offers them again. Nobody waits for the commit: the writer commits
    /// in queue order, after the authoritative mutation that changed the state.
    pub(crate) fn enqueue_outside_run_states(
        &self,
        session: Option<&str>,
        capture: impl FnOnce(&[EventTarget]) -> Option<Vec<RunState>>,
    ) -> Result<bool, DaemonError> {
        let followed = &self.outside_followed;
        // Held until the states are queued, so queue order is capture order.
        let mut cache = followed
            .cache
            .lock()
            .map_err(|_| error("outside follow cache poisoned"))?;
        let generation = followed.generation.load(Ordering::Acquire);
        if cache.generation != Some(generation) {
            *cache = Cache {
                generation: Some(generation),
                targets: self
                    .outside_parts()?
                    .into_iter()
                    .filter(|part| part.ended.is_none())
                    .flat_map(|part| part.targets)
                    .collect(),
                published: BTreeMap::new(),
            };
        }
        let targets: Vec<_> = cache
            .targets
            .iter()
            .filter(|target| session.is_none_or(|session| target.session_id == session))
            .cloned()
            .collect();
        if targets.is_empty() {
            return Ok(false);
        }
        let Some(mut states) = capture(&targets) else {
            return Ok(false);
        };
        states.retain(|state| {
            cache
                .published
                .get(&(state.session_id.clone(), state.agent_id.clone()))
                != Some(&published(state))
        });
        if states.is_empty() {
            return Ok(false);
        }
        drop(
            self.enqueue_agent_lifecycle(Operation::Outside(Mutation::RunStates {
                states: states.clone(),
            }))?,
        );
        for state in states {
            let value = published(&state);
            cache
                .published
                .insert((state.session_id, state.agent_id), value);
        }
        Ok(true)
    }
}
