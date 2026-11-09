//! MP-08 / MP-09 / MP-10 / MP-11: the Room terminal operations that need the gateway, which
//! only the router holds: running a held launch on another kernel, and listing where it could run.
use std::sync::Arc;
use std::time::Duration;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult,
};
use crate::room_exec::gateway_placement::OriginGrant;
use crate::room_exec::{destinations, offer_placement, ExecState};
use crate::runtime::command::KernelCaller;

/// A list never waits longer than this for the kernels a launch could move to.
const OFFER_WAIT: Duration = Duration::from_secs(2);

impl CommandRouter {
    pub(super) async fn room_terminal(
        &self,
        caller: &KernelCaller,
        request: RoomTerminalRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let state = &self.runtime_state;
        match &request.operation {
            RoomTerminalOperation::ExecPlace { kernel, .. } => {
                let placer = state.authorize_exec_place(caller, &request)?;
                let grant = Arc::new(OriginGrant::new(self.clone(), placer.origin, kernel));
                let result = state
                    .place_room_exec(
                        request,
                        &placer.requester,
                        &self.kernel_gateway().own_name(),
                        grant,
                    )
                    .await?;
                Ok(LocalDaemonResponse::RoomTerminal { result })
            }
            RoomTerminalOperation::ExecList => {
                let session_id = request.session_id.clone();
                let mut response = state.room_terminal_for_client(caller, request).await?;
                self.offer_placement(caller, &session_id, &mut response)
                    .await;
                Ok(response)
            }
            _ => state.room_terminal_for_client(caller, request).await,
        }
    }

    /// Names the kernels each held launch could run on, for the requester that may place it.
    /// A kernel list that cannot be had leaves the rows as they are: the list itself must not fail.
    async fn offer_placement(
        &self,
        caller: &KernelCaller,
        session_id: &str,
        response: &mut LocalDaemonResponse,
    ) {
        let LocalDaemonResponse::RoomTerminal {
            result: RoomTerminalResult::ExecListed { execs },
        } = response
        else {
            return;
        };
        if !execs
            .iter()
            .any(|exec| matches!(exec.state, ExecState::Queued { .. }))
        {
            return;
        }
        let asking = RoomTerminalRequest {
            session_id: session_id.to_owned(),
            acting_agent: None,
            host_agent: None,
            operation: RoomTerminalOperation::ExecPlace {
                exec_id: String::new(),
                kernel: String::new(),
                directory: None,
            },
        };
        let Ok(placer) = self.runtime_state.authorize_exec_place(caller, &asking) else {
            return;
        };
        let own_id = self.config_projection.snapshot().daemon_id;
        let listed = tokio::time::timeout(OFFER_WAIT, self.kernel_gateway().kernels()).await;
        let Ok(Ok(kernels)) = listed else {
            return;
        };
        let names = destinations(kernels, &own_id);
        offer_placement(execs, &placer.requester, &names);
    }
}
