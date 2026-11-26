//! Row 551: the agent's tool to restart a kernel (a hot reload of it). Parity with the owner's `RestartKernel` request: the same
//! preparation, the same refusal with its reason. With `kernel` it restarts another kernel of the owner through the gateway, as
//! `chariox kernels restart --kernel NAME` does: the same `RestartKernel` request, sent under the agent's own gateway grant.
use super::*;
use crate::local::{LocalDaemonRequest, RestartKernelRequest};
use crate::room_exec::gateway_placement::{GrantedSend, OriginGrant};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const TOOL: &str = "chariox.kernels.restart";

pub(crate) fn kernel_restart_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: TOOL.into(),
        description: "Restart this kernel as a hot reload: your terminals and running agents carry on, the Vault keeps its unlock and its deadline, \
                      and every client reconnects. The answer says the restart was accepted, or why it was refused and the kernel keeps running. \
                      Pass `kernel` (a name from `chariox kernels`) to restart another kernel of the owner instead."
            .into(),
        input_schema: serde_json::json!({"type":"object","properties":{"kernel":{"type":"string","minLength":1,"maxLength":128,"description":"Another kernel of the owner, by the name `chariox kernels` lists. Omit it to restart this kernel."}},"additionalProperties":false}),
    }
}

/// The restart of another kernel, sent through `grant` (the agent's gateway grant); the other kernel's own gate decides.
pub(crate) async fn restart_through(
    grant: &dyn GrantedSend,
    kernel: &str,
    command_id: String,
) -> Result<RuntimeToolResult, DaemonError> {
    let request = LocalDaemonRequest::RestartKernel(RestartKernelRequest {});
    let answer =
        grant
            .send(command_id, &request)
            .await
            .map_err(|failure| DaemonError::MachineRequest {
                code: failure.code().into(),
                message: failure.transport_error().message,
            })?;
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"restarting": true, "kernel": kernel, "answer": answer}),
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    kernel: Option<String>,
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
        let arguments: Arguments = serde_json::from_value(arguments).map_err(|_| {
            refuse(
                "invalid_request",
                "chariox.kernels.restart takes one optional argument, kernel: the name of another kernel of the owner",
            )
        })?;
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(refuse(
                "kernel_restart_owner_only",
                "Only the owner of this kernel or its agents can restart it.",
            ));
        };
        // This kernel or another one: the caller is a running agent of this kernel's owner, by the same reach check the
        // gateway grant is made from. An agent of another person running here, or a leased agent, has none.
        let reach = self
            .reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
            .ok_or_else(|| {
                refuse(
                    "kernel_restart_owner_only",
                    "Only a running agent of this kernel's owner can restart a kernel of the owner. Ask the owner to restart it (chariox kernels restart).",
                )
            })?;
        if let Some(kernel) = arguments.kernel {
            let command_id = format!("restart-kernel-{:032x}", rand::random::<u128>());
            #[cfg(test)]
            if let Some(grant) = super::job_compute::stand_in_kernel(&kernel) {
                return restart_through(&*grant, &kernel, command_id).await;
            }
            let router = self.job_router.as_ref().ok_or_else(|| {
                refuse(
                    "kernel_gateway_unavailable",
                    "The kernel gateway is unavailable on this kernel.",
                )
            })?;
            let grant = OriginGrant::new((**router).clone(), reach.origin(), &kernel);
            return restart_through(&grant, &kernel, command_id).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_exec::gateway_placement::SendFuture;
    use crate::runtime::kernel_gateway::GatewayError;

    /// The other kernel as the gateway reaches it: it answers a restart, or refuses it as the owner's gate would.
    struct OtherKernel(Result<serde_json::Value, (&'static str, &'static str)>);
    impl GrantedSend for OtherKernel {
        fn send<'a>(
            &'a self,
            _command_id: String,
            request: &'a LocalDaemonRequest,
        ) -> SendFuture<'a> {
            Box::pin(async move {
                assert!(
                    matches!(request, LocalDaemonRequest::RestartKernel(_)),
                    "the gateway carries the existing restart request, no new shape"
                );
                self.0
                    .clone()
                    .map_err(|(code, message)| GatewayError::Remote {
                        name: "other".into(),
                        code: code.into(),
                        message: message.into(),
                        retryable: false,
                    })
            })
        }
    }

    #[tokio::test]
    async fn an_owners_agent_restarts_a_second_owner_kernel_through_the_gateway_grant() {
        let other = OtherKernel(Ok(
            serde_json::json!({"KernelRestarting": {"hot_reload": true}}),
        ));
        let result = restart_through(&other, "second", "id-1".into())
            .await
            .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["kernel"], "second");
        assert_eq!(
            result.payload["answer"]["KernelRestarting"]["hot_reload"],
            true
        );
    }

    #[tokio::test]
    async fn another_owners_kernel_refuses_in_the_owner_gates_words() {
        let foreign = OtherKernel(Err((
            "kernel_restart_owner_only",
            "Only the owner of this kernel or its agents can restart it.",
        )));
        let refused = restart_through(&foreign, "foreign", "id-2".into())
            .await
            .unwrap_err();
        let text = refused.to_string();
        assert!(
            text.contains("Only the owner of this kernel or its agents can restart it."),
            "{text}"
        );
    }
}
