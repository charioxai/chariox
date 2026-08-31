use super::*;
use crate::local::{BindRoomEnvironmentSliceRequest, RoomEnvironmentSliceBinding};

impl KernelRuntimeState {
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
