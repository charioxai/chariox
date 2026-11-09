//! Who may run a held Room launch on another kernel, and under which gateway origin.
use std::sync::Arc;

use super::*;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult};
use crate::room_exec::gateway_placement::GrantedSend;
use crate::room_exec::{error, Placement, Requester};
use crate::runtime::command::KernelCaller;
use crate::runtime::kernel_gateway::Origin;
use crate::runtime::state::reach::{OWNER_GRANT_PREFIX, REACH_PREFIX};

pub(crate) struct ExecPlacer {
    pub(crate) requester: Requester,
    pub(crate) origin: Origin,
}

impl KernelRuntimeState {
    /// A person of the Room's owner places any held launch of the Room under the owner's grant.
    /// An agent places its own launches under its own live reach, like every other request it
    /// sends to another kernel.
    pub(crate) fn authorize_exec_place(
        &self,
        caller: &KernelCaller,
        request: &RoomTerminalRequest,
    ) -> Result<ExecPlacer, DaemonError> {
        self.authorize_room_terminal(caller, request, false)?;
        if request.host_agent.is_some() {
            return Err(error(
                "exec_place_not_here",
                "a held launch is placed by the kernel that holds it; leave host_agent out",
            ));
        }
        if caller.caller_id.starts_with(REACH_PREFIX) {
            let reach = self.reach_for_id(&caller.caller_id).ok_or_else(|| {
                error(
                    "exec_place_reach_ended",
                    "the agent's run is no longer live, so it holds no gateway grant; place the launch from a live run",
                )
            })?;
            if request
                .acting_agent
                .as_deref()
                .is_some_and(|agent| agent != reach.agent_id)
            {
                return Err(error(
                    "exec_not_yours",
                    "an agent places a launch only as itself",
                ));
            }
            return Ok(ExecPlacer {
                requester: Requester::Agent(reach.agent_id.clone()),
                origin: reach.origin(),
            });
        }
        if request.acting_agent.is_some() {
            return Err(error(
                "exec_not_yours",
                "an agent places a launch under its own live gateway grant: send the request through its reach tool",
            ));
        }
        let user_id = caller
            .user_id
            .as_deref()
            .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
        if !self.is_gateway_owner(user_id) {
            return Err(error(
                "exec_place_not_owner",
                "only this kernel's owner runs a launch on the owner's other kernels",
            ));
        }
        Ok(ExecPlacer {
            requester: Requester::Person,
            origin: Origin {
                grant_id: format!("{OWNER_GRANT_PREFIX}{user_id}"),
                harness: "Chariox".into(),
            },
        })
    }

    /// Runs the authorized `ExecPlace` through `grant`, the requester's own gateway grant.
    pub(crate) async fn place_room_exec(
        &self,
        request: RoomTerminalRequest,
        requester: &Requester,
        own_name: &str,
        grant: Arc<dyn GrantedSend>,
    ) -> Result<RoomTerminalResult, DaemonError> {
        let RoomTerminalOperation::ExecPlace {
            exec_id, directory, ..
        } = request.operation
        else {
            unreachable!("only ExecPlace is placed");
        };
        crate::room_exec::run_on_kernel(
            &self.owned.room_execs,
            Placement {
                session_id: &request.session_id,
                exec_id: &exec_id,
                directory: directory.as_deref(),
                origin_kernel: own_name,
            },
            requester,
            grant,
        )
        .await?;
        Ok(RoomTerminalResult::ExecOpened { exec_id })
    }
}
