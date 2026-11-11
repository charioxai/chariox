//! The live setup of each new Room. Process-local on purpose: a setup interrupted by a restart
//! is not resumed, and a Room never waits on a setup that no longer runs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::room_setup::{RoomSetupRequest, RoomSetupState, RoomSetupStepId, RoomSetupStepState};
use crate::error::DaemonError;

#[derive(Debug, Clone, Default)]
pub(crate) struct RoomSetupStore {
    rooms: Arc<Mutex<BTreeMap<String, RoomSetup>>>,
}

#[derive(Debug)]
struct RoomSetup {
    agent_id: String,
    state: RoomSetupState,
}

impl RoomSetupStore {
    fn rooms(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, RoomSetup>> {
        self.rooms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Registers the steps a new Room was asked to run on its first agent, all pending.
    pub(crate) fn begin(&self, session_id: &str, agent_id: &str, request: &RoomSetupRequest) {
        if let Some(state) = RoomSetupState::pending(request) {
            self.rooms().insert(
                session_id.to_string(),
                RoomSetup {
                    agent_id: agent_id.to_string(),
                    state,
                },
            );
        }
    }

    pub(crate) fn state(&self, session_id: &str) -> Option<RoomSetupState> {
        self.rooms()
            .get(session_id)
            .map(|setup| setup.state.clone())
    }

    /// Whether prompts to the Room wait: a step is unfinished, or `git_init` or `worktree`
    /// failed and was not dismissed.
    pub(crate) fn holds_prompts(&self, session_id: &str) -> bool {
        self.rooms()
            .get(session_id)
            .is_some_and(|setup| setup.state.holds_prompts())
    }

    pub(crate) fn start(&self, session_id: &str, step: RoomSetupStepId) {
        self.set_step(session_id, step, RoomSetupStepState::Running, None);
    }

    /// Ends a step. A failed `git_init` or `worktree` drops the steps after it: they need its
    /// result. When every step is done the Room's setup is cleared.
    pub(crate) fn finish(
        &self,
        session_id: &str,
        step: RoomSetupStepId,
        result: Result<(), String>,
    ) {
        let (state, message) = match result {
            Ok(()) => (RoomSetupStepState::Done, None),
            Err(message) => (RoomSetupStepState::Failed, Some(message)),
        };
        let mut rooms = self.rooms();
        let Some(setup) = rooms.get_mut(session_id) else {
            return;
        };
        apply_step(&mut setup.state, step, state, message);
        if state == RoomSetupStepState::Failed && step != RoomSetupStepId::Publish {
            let position = setup.state.steps.iter().position(|item| item.id == step);
            if let Some(position) = position {
                setup.state.steps.truncate(position + 1);
            }
        }
        if setup.state.is_complete() {
            rooms.remove(session_id);
        }
    }

    fn set_step(
        &self,
        session_id: &str,
        step: RoomSetupStepId,
        state: RoomSetupStepState,
        message: Option<String>,
    ) {
        if let Some(setup) = self.rooms().get_mut(session_id) {
            apply_step(&mut setup.state, step, state, message);
        }
    }

    /// Drops a Room's setup without running it further.
    pub(crate) fn clear(&self, session_id: &str) {
        self.rooms().remove(session_id);
    }

    /// The agent of a failed `publish` step, which becomes pending again.
    pub(crate) fn retry_publish(&self, session_id: &str) -> Result<String, DaemonError> {
        let mut rooms = self.rooms();
        let failed_publish = rooms.get_mut(session_id).and_then(|setup| {
            let step = setup.state.steps.iter_mut().find(|step| {
                step.id == RoomSetupStepId::Publish && step.state == RoomSetupStepState::Failed
            })?;
            step.state = RoomSetupStepState::Pending;
            step.message = None;
            Some(setup.agent_id.clone())
        });
        failed_publish.ok_or_else(|| DaemonError::LocalTransport {
            operation: "retry room setup",
            message: "no publish step failed in this room; there is nothing to retry".to_string(),
        })
    }

    /// Clears a failed setup. Returns whether anything was cleared.
    pub(crate) fn dismiss(&self, session_id: &str) -> bool {
        let mut rooms = self.rooms();
        if rooms
            .get(session_id)
            .is_some_and(|setup| setup.state.has_failed())
        {
            rooms.remove(session_id);
            return true;
        }
        false
    }
}

fn apply_step(
    state: &mut RoomSetupState,
    id: RoomSetupStepId,
    next: RoomSetupStepState,
    message: Option<String>,
) {
    if let Some(step) = state.steps.iter_mut().find(|step| step.id == id) {
        step.state = next;
        step.message = message;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn everything() -> RoomSetupRequest {
        RoomSetupRequest {
            git_init: true,
            new_worktree: true,
            publish: true,
        }
    }

    #[test]
    fn nothing_requested_registers_nothing() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &RoomSetupRequest::default());
        assert_eq!(store.state("room"), None);
        assert!(!store.holds_prompts("room"));
    }

    #[test]
    fn steps_hold_prompts_until_done_then_clear() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything());
        assert!(store.holds_prompts("room"));
        for step in [
            RoomSetupStepId::GitInit,
            RoomSetupStepId::Worktree,
            RoomSetupStepId::Publish,
        ] {
            assert!(store.holds_prompts("room"));
            store.start("room", step);
            store.finish("room", step, Ok(()));
        }
        assert_eq!(store.state("room"), None);
        assert!(!store.holds_prompts("room"));
    }

    #[test]
    fn failed_worktree_holds_until_dismissed_and_drops_later_steps() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything());
        store.finish("room", RoomSetupStepId::GitInit, Ok(()));
        store.finish("room", RoomSetupStepId::Worktree, Err("no commits".into()));
        let state = store.state("room").unwrap();
        assert_eq!(state.steps.len(), 2);
        assert!(store.holds_prompts("room"));
        assert!(store.dismiss("room"));
        assert!(!store.holds_prompts("room"));
        assert_eq!(store.state("room"), None);
    }

    #[test]
    fn failed_publish_never_holds_and_retries_as_pending() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything());
        store.finish("room", RoomSetupStepId::GitInit, Ok(()));
        store.finish("room", RoomSetupStepId::Worktree, Ok(()));
        store.finish(
            "room",
            RoomSetupStepId::Publish,
            Err("not logged in".into()),
        );
        assert!(!store.holds_prompts("room"));
        assert!(store.state("room").unwrap().has_failed());
        assert_eq!(store.retry_publish("room").unwrap(), "agent");
        assert!(store.holds_prompts("room"));
        assert!(!store.state("room").unwrap().has_failed());
        assert!(store.retry_publish("room").is_err());
    }

    #[test]
    fn dismiss_without_failure_changes_nothing() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything());
        assert!(!store.dismiss("room"));
        assert!(store.holds_prompts("room"));
    }
}
