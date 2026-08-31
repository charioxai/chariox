use super::*;
use crate::local::{BindRoomEnvironmentSliceRequest, RoomEnvironmentSliceBinding};

pub(crate) struct SliceExecutionAdmission {
    // Preserve target order, including non-slice workers and duplicate aliases.
    // The guarded identity must also be used for the successful attachment.
    pub(crate) slice_ids: Vec<Option<String>>,
    _guards: Vec<crate::slice::SliceOperationGuard>,
}

impl KernelRuntimeState {
    pub(crate) fn guard_slice_execution<'a>(
        &self,
        session_id: Option<&str>,
        targets: impl IntoIterator<Item = (Option<&'a str>, Option<&'a str>)>,
        operation: &'static str,
    ) -> Result<SliceExecutionAdmission, DaemonError> {
        let slices = &self.owned.slice_store;
        let mut unique_ids = std::collections::BTreeSet::new();
        let mut slice_ids = Vec::new();
        for (slice_ref, kernel_ref) in targets {
            let slice = match (slice_ref, kernel_ref) {
                (Some(_), Some(_)) => {
                    return Err(DaemonError::LocalTransport {
                        operation,
                        message: "use either kernel_ref or slice_ref, not both".to_string(),
                    });
                }
                (Some(reference), None) => Some(slices.resolve(reference)?),
                (None, Some(reference)) => slices.resolve_by_worker_kernel_ref(reference),
                (None, None) => None,
            };
            let slice_id = slice.map(|slice| slice.id);
            if let Some(id) = &slice_id {
                unique_ids.insert(id.clone());
            }
            slice_ids.push(slice_id);
        }
        let guards = unique_ids
            .iter()
            .map(|id| slices.guard_environment_use(id, session_id, operation))
            .collect::<Result<_, _>>()?;
        Ok(SliceExecutionAdmission {
            slice_ids,
            _guards: guards,
        })
    }

    pub(crate) fn room_environment_slice(
        &self,
        session_id: &str,
    ) -> Result<Option<RoomEnvironmentSliceBinding>, DaemonError> {
        self.owned.session_store.get_session(session_id)?;
        Ok(self
            .owned
            .slice_store
            .environment_slice(session_id)
            .map(|slice| binding(session_id, slice)))
    }

    pub(crate) fn bind_room_environment_slice(
        &self,
        request: BindRoomEnvironmentSliceRequest,
        caller_user_id: &str,
    ) -> Result<RoomEnvironmentSliceBinding, DaemonError> {
        let sessions = self.owned.session_store.read();
        let session = sessions.get_session(&request.session_id)?;
        if session.owner_user_id() != caller_user_id {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: caller_user_id.to_string(),
                owner_user_id: session.owner_user_id().to_string(),
                resource: request.session_id.clone(),
                operation: "environment.slice.bind",
            });
        }
        if sessions.is_ephemeral_session(&request.session_id) {
            return Err(DaemonError::LocalTransport {
                operation: "environment.slice.bind",
                message: "environment_slice_binding_rejected: a durable Environment requires a durable Room".to_string(),
            });
        }
        let slice = self.owned.slice_store.bind_environment(
            &request.session_id,
            &request.slice_ref,
            crate::session::unix_epoch_ms(),
            |slice| {
                self.owned.durable_state_store.append_event(
                    "slice.updated",
                    Some(slice.id.clone()),
                    serde_json::json!({"slice": slice}),
                )?;
                Ok(())
            },
        )?;
        self.owned.runtime_projection_changes.record_change();
        Ok(binding(&request.session_id, slice))
    }
}

fn binding(session_id: &str, slice: crate::slice::SliceRecord) -> RoomEnvironmentSliceBinding {
    RoomEnvironmentSliceBinding {
        session_id: session_id.to_string(),
        slice_id: slice.id,
        owner_kernel_id: slice.owner_kernel_id,
        worker_kernel_ref: slice.worker_kernel_ref,
    }
}
