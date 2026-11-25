//! Row 551: the agent's tool to restart its own kernel (a hot reload of it). Parity with the owner's `RestartKernel` request: the same
//! preparation, the same refusal with its reason. Another kernel of the owner is restarted with `chariox kernels restart --kernel NAME`.
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const TOOL: &str = "chariox.kernels.restart";

pub(crate) fn kernel_restart_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: TOOL.into(),
        description: "Restart this kernel as a hot reload: your terminals and running agents carry on, the Vault keeps its unlock and its deadline, \
                      and every client reconnects. The answer says the restart was accepted, or why it was refused and the kernel keeps running. \
                      It takes no arguments; restart another kernel of the owner with `chariox kernels restart --kernel NAME`."
            .into(),
        input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn kernel_restart_tool(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let refuse = |code: &str, message: &str| DaemonError::MachineRequest {
            code: code.into(),
            message: message.into(),
        };
        if arguments
            .as_object()
            .is_some_and(|fields| !fields.is_empty())
        {
            return Err(refuse(
                "invalid_request",
                "chariox.kernels.restart takes no arguments; restart another kernel of the owner with `chariox kernels restart --kernel NAME`",
            ));
        }
        if self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token)
            .is_empty()
        {
            return Err(refuse(
                "kernel_restart_owner_only",
                "Only the owner of this kernel or its agents can restart it.",
            ));
        }
        crate::hot_reload::request_restart()
            .await
            .map_err(|reason| restart_refused(&reason))?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"restarting": true, "hot_reload": true}),
        })
    }
}
