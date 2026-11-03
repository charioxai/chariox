//! MP-08 / MP-09 / MP-10 / MP-11 reach: the `chariox-reach` tools of a kernel owner's
//! Room agent. Each call resolves the agent's own live run again, so ending the run
//! ends the reach. Another kernel is asked through the owner's gateway, naming the agent.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::kernel_gateway::Origin;
use crate::runtime::state::reach::{acquires_machine, reach_need, ReachCaller, ReachNeed};
use crate::transport::runtime_tools::{
    ReachRequestArgs, RuntimeToolResult, REACH_KERNELS_TOOL, REACH_REQUEST_TOOL,
};

/// A refused call is a tool result with a stable code, not a transport failure.
fn refused(code: &str, message: impl Into<String>) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({"code": code, "message": message.into()}),
    }
}

impl CommandRouter {
    pub(crate) fn reach_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        match self.runtime_state.reach_for_auth_token(auth_token) {
            Ok(_) => crate::transport::runtime_tools::reach_tool_specs(),
            Err(_) => Vec::new(),
        }
    }

    pub(crate) async fn dispatch_reach_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let caller = match self.runtime_state.reach_for_auth_token(auth_token) {
            Ok(caller) => caller,
            Err(error) => return Ok(refused("reach_not_available", error.to_string())),
        };
        match tool_name {
            REACH_KERNELS_TOOL => Ok(self.reach_kernels().await),
            REACH_REQUEST_TOOL => {
                let args: ReachRequestArgs =
                    serde_json::from_value(arguments).map_err(|error| {
                        crate::runtime::kernel_access::error(format!(
                            "invalid request tool arguments: {error}"
                        ))
                    })?;
                let request: LocalDaemonRequest =
                    serde_json::from_value(args.request).map_err(|_| {
                        crate::runtime::kernel_access::error("invalid kernel request: use externally tagged JSON in request, for example {\"ListSessions\":null}")
                    })?;
                self.reach_request(&caller, args.kernel, request).await
            }
            _ => Ok(refused("reach_unknown_tool", "no such chariox-reach tool")),
        }
    }

    async fn reach_kernels(&self) -> RuntimeToolResult {
        match self.kernel_gateway().kernels().await {
            Ok(kernels) => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"kernels": kernels}),
            },
            Err(failure) => refused(failure.code(), failure.transport_error().message),
        }
    }

    async fn reach_request(
        &self,
        caller: &ReachCaller,
        kernel: Option<String>,
        request: LocalDaemonRequest,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let state = &self.runtime_state;
        // Without a /sudo window of this agent's own turn, only what a Room agent
        // may do in its own room; the window adds what /sudo adds at home.
        let sudo =
            match reach_need(&request) {
                ReachNeed::Room => None,
                ReachNeed::HumanOnly => {
                    return Ok(refused(
                        "human_only",
                        "this request belongs to the owner and no agent may send it",
                    ))
                }
                ReachNeed::Excluded => {
                    return Ok(refused(
                        "excluded_for_agents",
                        "agents may not send this request, not even with /sudo",
                    ))
                }
                ReachNeed::Sudo => match state.reach_sudo_turn(caller) {
                    Some(turn) => Some(turn),
                    None => return Ok(refused(
                        "reach_requires_sudo",
                        "this needs /sudo in your own room too; ask the owner to /sudo your task",
                    )),
                },
            };
        if acquires_machine(&request) && !state.confirm_machine_acquisition(caller).await? {
            return Ok(refused(
                "acquisition_declined",
                "the owner did not accept acquiring a machine",
            ));
        }
        let command_id = format!("{}:{}", caller.id, rand::random::<u64>());
        let Some(kernel) = kernel else {
            return match sudo {
                Some(turn) => {
                    self.dispatch_sudo_kernel_request(turn, serde_json::json!({"request": request}))
                        .await
                }
                None => self.reach_local(caller, command_id, request).await,
            };
        };
        let origin = match &sudo {
            Some(turn) => Origin {
                grant_id: turn.entry_id.clone(),
                harness: caller.harness.clone(),
            },
            None => Origin {
                grant_id: caller.id.clone(),
                harness: caller.harness.clone(),
            },
        };
        if let Err(error) = state.authorize_gateway_request(&origin.grant_id, &request) {
            return Ok(refused("not_allowed", error.to_string()));
        }
        Ok(
            match crate::runtime::kernel_gateway::forward_audited(
                self,
                &origin,
                &command_id,
                &kernel,
                &request,
            )
            .await
            {
                Ok(response) => RuntimeToolResult {
                    ok: true,
                    payload: response,
                },
                Err(failure) => refused(failure.code(), failure.transport_error().message),
            },
        )
    }

    /// The agent's own kernel: the owner's request, named after the run, with none of
    /// the room fences a Room tool call carries.
    async fn reach_local(
        &self,
        caller: &ReachCaller,
        command_id: String,
        request: LocalDaemonRequest,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let mut command = crate::runtime::command::KernelCommand::from_local_request(
            command_id,
            None,
            Some(caller.id.clone()),
            &request,
        );
        command.caller = crate::runtime::command::KernelCaller::for_source(
            &crate::runtime::command::KernelCommandSource::LocalIpc,
        )
        .with_connection_class(crate::local::KernelConnectionClass::KernelAgent);
        command.provider_run_id = Some(caller.run_id.clone());
        command.caller.caller_id = caller.id.clone();
        command.caller.user_id = Some(caller.owner_user_id.clone());
        let response = match Box::pin(self.dispatch(command, request)).await {
            Ok(response) => response,
            Err(error) => return Ok(refused("request_failed", error.to_string())),
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(response).map_err(|_| {
                crate::runtime::kernel_access::error("kernel response serialization failed")
            })?,
        })
    }
}
