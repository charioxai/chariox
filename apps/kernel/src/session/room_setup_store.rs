//! Durable setup of each new Room. Interrupted work becomes an explicit failure on restart.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::room_setup::{RoomSetupRequest, RoomSetupState, RoomSetupStepId, RoomSetupStepState};
use crate::error::DaemonError;

#[derive(Debug, Clone, Default)]
pub(crate) struct RoomSetupStore {
    rooms: Arc<Mutex<BTreeMap<String, RoomSetup>>>,
    durable: Option<(crate::durable_state::DurableKernelStateStore, String)>,
}

#[derive(Debug, Clone)]
struct RoomSetup {
    agent_id: String,
    state: RoomSetupState,
    storage_failed: bool,
}

impl RoomSetup {
    fn fail_step(&mut self, position: usize, message: &'static str) {
        let step = &mut self.state.steps[position];
        step.state = RoomSetupStepState::Failed;
        step.message = Some(message.into());
        self.state.steps.truncate(position + 1);
    }

    fn storage_failed(&mut self, position: usize) {
        self.fail_step(
            position,
            storage_failure_message(self.state.steps[position].id),
        );
        self.storage_failed = true;
    }
}

impl RoomSetupStore {
    pub(crate) fn durable(
        store: crate::durable_state::DurableKernelStateStore,
        owner: &str,
    ) -> Result<Self, DaemonError> {
        let restored = Self {
            rooms: Default::default(),
            durable: Some((store.clone(), owner.into())),
        };
        let mut rooms = restored.rooms();
        for saved in store.load_room_setups(owner)? {
            let session_id = saved.session_id;
            let mut setup = RoomSetup {
                agent_id: saved.agent_id,
                state: saved.state,
                storage_failed: saved.storage_failed,
            };
            // No setup task survives the process. The last committed state identifies the
            // first unfinished step even when recording the result was the write that failed.
            if let Some(position) = setup.state.steps.iter().position(|step| {
                matches!(
                    step.state,
                    RoomSetupStepState::Pending | RoomSetupStepState::Running
                )
            }) {
                if setup.storage_failed {
                    setup.storage_failed(position);
                } else {
                    let step = setup.state.steps[position].id;
                    setup.fail_step(position, interrupted_message(step));
                    if let Err(error) = store.save_room_setup(
                        owner,
                        &session_id,
                        Some((&setup.agent_id, &setup.state)),
                    ) {
                        tracing::error!(session_id, %error, "Room setup recovery could not save progress; repair kernel storage, then retry or dismiss setup");
                        restored.record_storage_failure(&session_id);
                        setup.storage_failed(position);
                    }
                }
            }
            rooms.insert(session_id, setup);
        }
        drop(rooms);
        Ok(restored)
    }

    fn change(
        &self,
        session_id: &str,
        update: impl FnOnce(Option<RoomSetup>) -> Result<Option<RoomSetup>, DaemonError>,
    ) -> Result<(), DaemonError> {
        let mut rooms = self.rooms();
        let next = update(rooms.get(session_id).cloned())?;
        if let Some((store, owner)) = &self.durable {
            store.save_room_setup(
                owner,
                session_id,
                next.as_ref()
                    .map(|setup| (setup.agent_id.as_str(), &setup.state)),
            )?;
        }
        if let Some(next) = next {
            rooms.insert(session_id.into(), next);
        } else {
            rooms.remove(session_id);
        }
        Ok(())
    }

    fn rooms(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, RoomSetup>> {
        self.rooms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Registers the steps a new Room was asked to run on its first agent, all pending.
    pub(crate) fn begin(
        &self,
        session_id: &str,
        agent_id: &str,
        request: &RoomSetupRequest,
    ) -> Result<(), DaemonError> {
        self.change(session_id, |_| {
            Ok(RoomSetupState::pending(request).map(|state| RoomSetup {
                agent_id: agent_id.into(),
                state,
                storage_failed: false,
            }))
        })
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
            .is_some_and(|setup| !setup.storage_failed && setup.state.holds_prompts())
    }

    /// Project the failure and release prompts even if storage cannot record the failure marker.
    /// Retry or Dismiss must commit normally after storage is repaired.
    pub(crate) fn progress_failed(&self, session_id: &str, step: RoomSetupStepId) {
        let mut rooms = self.rooms();
        let Some(setup) = rooms.get_mut(session_id) else {
            return;
        };
        if let Some(position) = setup.state.steps.iter().position(|item| item.id == step) {
            self.record_storage_failure(session_id);
            setup.storage_failed(position);
        }
    }

    fn record_storage_failure(&self, session_id: &str) {
        if let Some((store, owner)) = &self.durable {
            // The progress write may still be rejected. Save only the storage failure marker.
            if let Err(error) = store.mark_room_setup_storage_failure(owner, session_id) {
                tracing::error!(session_id, %error, "Room setup storage failure could not be recorded");
            }
        }
    }

    pub(crate) fn start(&self, session_id: &str, step: RoomSetupStepId) -> Result<(), DaemonError> {
        self.change(session_id, |setup| {
            Ok(setup.map(|mut setup| {
                apply_step(&mut setup.state, step, RoomSetupStepState::Running, None);
                setup
            }))
        })
    }

    /// Ends a step. A failed `git_init` or `worktree` drops the steps after it: they need its
    /// result. When every step is done the Room's setup is cleared.
    pub(crate) fn finish(
        &self,
        session_id: &str,
        step: RoomSetupStepId,
        result: Result<(), String>,
    ) -> Result<(), DaemonError> {
        let (state, message) = match result {
            Ok(()) => (RoomSetupStepState::Done, None),
            Err(message) => (RoomSetupStepState::Failed, Some(message)),
        };
        self.change(session_id, |setup| {
            Ok(setup.and_then(|mut setup| {
                apply_step(&mut setup.state, step, state, message);
                if state == RoomSetupStepState::Failed && step != RoomSetupStepId::Publish {
                    if let Some(position) =
                        setup.state.steps.iter().position(|item| item.id == step)
                    {
                        setup.state.steps.truncate(position + 1);
                    }
                }
                (!setup.state.is_complete()).then_some(setup)
            }))
        })
    }

    /// Drops a Room's setup without running it further.
    pub(crate) fn clear(&self, session_id: &str) -> Result<(), DaemonError> {
        self.change(session_id, |_| Ok(None))
    }

    /// The agent of a failed `publish` step, which becomes pending again.
    pub(crate) fn retry_publish(&self, session_id: &str) -> Result<String, DaemonError> {
        let mut agent = None;
        self.change(session_id, |setup| {
            let mut setup = setup.ok_or_else(nothing_to_retry)?;
            let step = setup
                .state
                .steps
                .iter_mut()
                .find(|step| {
                    step.id == RoomSetupStepId::Publish && step.state == RoomSetupStepState::Failed
                })
                .ok_or_else(nothing_to_retry)?;
            step.state = RoomSetupStepState::Pending;
            step.message = None;
            setup.storage_failed = false;
            agent = Some(setup.agent_id.clone());
            Ok(Some(setup))
        })?;
        Ok(agent.expect("retry records its agent after a successful commit"))
    }

    /// Clears a failed setup. Returns whether anything was cleared.
    pub(crate) fn dismiss(&self, session_id: &str) -> Result<bool, DaemonError> {
        let mut dismissed = false;
        self.change(session_id, |setup| {
            Ok(setup.filter(|setup| {
                dismissed = setup.state.has_failed();
                !dismissed
            }))
        })?;
        Ok(dismissed)
    }
}

fn storage_failure_message(step: RoomSetupStepId) -> &'static str {
    if step == RoomSetupStepId::Publish {
        "Publishing progress could not be saved. Repair kernel storage and check the repository, then retry publishing."
    } else {
        "Room setup progress could not be saved. Repair kernel storage and check the folder, then dismiss setup to continue."
    }
}

fn interrupted_message(step: RoomSetupStepId) -> &'static str {
    if step == RoomSetupStepId::Publish {
        "Publishing was interrupted by a kernel restart; retry publishing"
    } else {
        "Room setup was interrupted by a kernel restart; check the folder, then dismiss to continue"
    }
}

fn nothing_to_retry() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "retry room setup",
        message: "no publish step failed in this room; there is nothing to retry".into(),
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
        store
            .begin("room", "agent", &RoomSetupRequest::default())
            .unwrap();
        assert_eq!(store.state("room"), None);
        assert!(!store.holds_prompts("room"));
    }

    #[test]
    fn steps_hold_prompts_until_done_then_clear() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything()).unwrap();
        assert!(store.holds_prompts("room"));
        for step in [
            RoomSetupStepId::GitInit,
            RoomSetupStepId::Worktree,
            RoomSetupStepId::Publish,
        ] {
            assert!(store.holds_prompts("room"));
            store.start("room", step).unwrap();
            store.finish("room", step, Ok(())).unwrap();
        }
        assert_eq!(store.state("room"), None);
        assert!(!store.holds_prompts("room"));
    }

    #[test]
    fn failed_worktree_holds_until_dismissed_and_drops_later_steps() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything()).unwrap();
        store
            .finish("room", RoomSetupStepId::GitInit, Ok(()))
            .unwrap();
        store
            .finish("room", RoomSetupStepId::Worktree, Err("no commits".into()))
            .unwrap();
        let state = store.state("room").unwrap();
        assert_eq!(state.steps.len(), 2);
        assert!(store.holds_prompts("room"));
        assert!(store.dismiss("room").unwrap());
        assert!(!store.holds_prompts("room"));
        assert_eq!(store.state("room"), None);
    }

    #[test]
    fn failed_publish_never_holds_and_retries_as_pending() {
        let store = RoomSetupStore::default();
        store.begin("room", "agent", &everything()).unwrap();
        store
            .finish("room", RoomSetupStepId::GitInit, Ok(()))
            .unwrap();
        store
            .finish("room", RoomSetupStepId::Worktree, Ok(()))
            .unwrap();
        store
            .finish(
                "room",
                RoomSetupStepId::Publish,
                Err("not logged in".into()),
            )
            .unwrap();
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
        store.begin("room", "agent", &everything()).unwrap();
        assert!(!store.dismiss("room").unwrap());
        assert!(store.holds_prompts("room"));
    }
    #[test]
    fn durable_failure_retries_on_its_original_agent_and_completion_stays_cleared() {
        let root = crate::test_temp_dir::TestTempDir::new("room-setup-durable");
        let db =
            crate::durable_state::DurableKernelStateStore::open(root.join("kernel.db")).unwrap();
        let setup = RoomSetupRequest {
            publish: true,
            ..Default::default()
        };
        let store = RoomSetupStore::durable(db.clone(), "kernel").unwrap();
        store.begin("room", "first-agent", &setup).unwrap();
        store
            .finish("room", RoomSetupStepId::Publish, Err("no login".into()))
            .unwrap();
        let failed = store.state("room");
        drop(store);
        assert_eq!(
            RoomSetupStore::durable(db.clone(), "other-kernel")
                .unwrap()
                .state("room"),
            None
        );
        let restored = RoomSetupStore::durable(db.clone(), "kernel").unwrap();
        assert_eq!(restored.state("room"), failed);
        assert_eq!(restored.retry_publish("room").unwrap(), "first-agent");
        restored
            .finish("room", RoomSetupStepId::Publish, Ok(()))
            .unwrap();
        drop(restored);
        assert_eq!(
            RoomSetupStore::durable(db, "kernel").unwrap().state("room"),
            None
        );
    }

    #[test]
    fn interrupted_steps_fail_without_waiting_on_a_dead_task_and_dismiss_stays_cleared() {
        let root = crate::test_temp_dir::TestTempDir::new("room-setup-interrupted");
        let db =
            crate::durable_state::DurableKernelStateStore::open(root.join("kernel.db")).unwrap();
        let store = RoomSetupStore::durable(db.clone(), "kernel").unwrap();
        store.begin("room", "agent", &everything()).unwrap();
        store.start("room", RoomSetupStepId::GitInit).unwrap();
        drop(store);
        let restored = RoomSetupStore::durable(db.clone(), "kernel").unwrap();
        let state = restored.state("room").unwrap();
        assert_eq!(state.steps.len(), 1);
        assert_eq!(state.steps[0].state, RoomSetupStepState::Failed);
        assert!(state.steps[0]
            .message
            .as_ref()
            .unwrap()
            .contains("interrupted"));
        assert!(restored.dismiss("room").unwrap());
        drop(restored);
        assert_eq!(
            RoomSetupStore::durable(db, "kernel").unwrap().state("room"),
            None
        );
    }
}
