//! Who may run a held Room launch on another kernel, and under which gateway origin.
use super::*;
use crate::local::RoomTerminalRequest;
use crate::room_exec::Requester;
use crate::runtime::command::KernelCaller;
use crate::runtime::kernel_gateway::Origin;

pub(crate) struct ExecPlacer {
    pub(crate) requester: Requester,
    pub(crate) origin: Origin,
}

impl KernelRuntimeState {
    pub(crate) fn authorize_exec_place(
        &self,
        _caller: &KernelCaller,
        _request: &RoomTerminalRequest,
    ) -> Result<ExecPlacer, DaemonError> {
        Err(crate::room_exec::error(
            "exec_place_unavailable",
            "running a held launch on another kernel is not wired yet",
        ))
    }
}
