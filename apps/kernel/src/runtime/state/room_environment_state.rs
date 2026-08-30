use crate::session::{EnvironmentError, RoomEnvironmentSnapshot};

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) fn room_environment_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_snapshot(session_id)
    }
}
