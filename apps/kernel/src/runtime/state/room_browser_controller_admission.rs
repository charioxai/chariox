use super::*;
use crate::slice::{SliceEnvironmentUseGuard, SliceRecord, SliceStatus};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand as Command;

impl KernelRuntimeState {
    pub(crate) async fn admit_room_browser_controller_route(
        &self,
        session_id: &str,
        slice_id: &str,
        command: &Command,
        background_probe: bool,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<(SliceRecord, Option<SliceEnvironmentUseGuard>), DaemonError> {
        debug_assert!(!background_probe || matches!(command, Command::Reconcile { .. }));
        // Cancellation bypasses the slot held by its original action. App
        // bridge polls/answers only drain a queue and may share a route slot.
        let guard = if matches!(command, Command::CancelAction { .. }) {
            None
        } else if background_probe || matches!(
            command,
            Command::AppView {
                request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Calls
                    | crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Respond { .. }
            }
        ) {
            self.owned.slice_store.check_shared_environment_use(
                slice_id,
                Some(session_id),
                "browser_controller.route",
                "browser_controller.route",
            )?;
            None
        } else if matches!(
            command,
            Command::ComputerInput { .. }
                | Command::ComputerClipboardRead { .. }
                | Command::Acquire
                | Command::Reconcile { .. }
                | Command::Snapshot { .. }
                | Command::Artifact { .. }
                | Command::Wait { .. }
                | Command::PollEvents { .. }
        ) {
            Some(
                self.owned
                    .slice_store
                    .queue_environment_use_until(
                        slice_id,
                        Some(session_id),
                        "browser_controller.route",
                        deadline.unwrap_or_else(|| {
                            tokio::time::Instant::now()
                                + crate::slice::ENVIRONMENT_USE_ADMISSION_TIMEOUT
                        }),
                    )
                    .await?,
            )
        } else {
            // Browser mutations retain their existing admission/cancellation
            // contract. Viewer input and recurring reads queue behind them.
            Some(
                self.owned
                    .slice_store
                    .guard_environment_use(slice_id, Some(session_id), "browser_controller.route")?
                    .into(),
            )
        };
        // Re-read after admission, before relay I/O. Offline retries must not
        // hold the lifecycle slot while waiting for a stopped worker.
        let slice = self.owned.slice_store.resolve(slice_id)?;
        if slice.status != SliceStatus::Running {
            return Err(admission_error(
                "browser_controller_unavailable: slice is not running",
            ));
        }
        if let Command::ComputerInput {
            action_id,
            actor_id,
            runtime_generation,
            viewport_revision,
            ..
        } = command
        {
            self.require_room_computer_actor(actor_id)?;
            let room = self
                .room_environment_snapshot(session_id)
                .map_err(|_| admission_error("Room input state is unavailable before dispatch"))?;
            let action = room
                .actions
                .iter()
                .find(|action| action.action_id == *action_id);
            if action.is_none_or(|action| {
                action.cancellation_requested
                    || action.state != crate::session::EnvironmentActionState::Running
                    || action.actor_id != *actor_id
            }) {
                return Err(DaemonError::BrowserControllerActionCancelled {
                    controller_fenced: false,
                });
            }
            if room.runtime_generation != *runtime_generation
                || room.viewport.revision != *viewport_revision
            {
                return Err(admission_error(
                    "Room input generation or viewport changed while waiting for dispatch",
                ));
            }
            if room.actors.iter().any(|actor| {
                actor.actor_id == *actor_id
                    && actor.kind == crate::session::EnvironmentActorKind::Human
            }) && !room.input_ownership.iter().any(|owner| {
                owner.target == crate::session::InputTarget::Desktop && owner.actor_id == *actor_id
            }) {
                return Err(admission_error(
                    "Room human input ownership changed while waiting for dispatch",
                ));
            }
            self.ensure_browser_import_execution_allowed(session_id)
                .map_err(browser_import_execution_gate::execution_error)?;
        }
        if let Command::ComputerClipboardRead {
            actor_id,
            runtime_generation,
        } = command
        {
            let room = self.room_environment_snapshot(session_id).map_err(|_| {
                admission_error("Room clipboard state is unavailable before dispatch")
            })?;
            if room.runtime_generation != *runtime_generation
                || !room.input_ownership.iter().any(|owner| {
                    owner.target == crate::session::InputTarget::Desktop
                        && owner.actor_id == *actor_id
                })
            {
                return Err(admission_error(
                    "Room clipboard authority changed while waiting for dispatch",
                ));
            }
        }
        Ok((slice, guard))
    }
}

fn admission_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.to_string(),
    }
}

// MD-N2: refuse before serializing a new command to an old or unknown worker.
pub(super) fn require_notes_worker_protocol(version: Option<u32>) -> Result<(), DaemonError> {
    if version
        .is_some_and(|version| version >= crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION)
    {
        Ok(())
    } else {
        Err(admission_error(&format!(
            "Room Notes require relay peer protocol {} or newer",
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        )))
    }
}
#[cfg(test)]
mod notes_peer_tests {
    #[test]
    fn md_notes_refuses_unknown_and_pre_union_workers_before_dispatch() {
        let minimum = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        for version in [None, Some(70), Some(73), Some(minimum - 1)] {
            assert!(super::require_notes_worker_protocol(version).is_err());
        }
        for version in [Some(minimum), Some(minimum + 1)] {
            assert!(super::require_notes_worker_protocol(version).is_ok());
        }
    }
}
