use super::*;

impl CommandRouter {
    pub(super) async fn kernel_identity_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Option<RuntimeToolResult> {
        if tokens.first().is_none_or(|word| word != "kernel") {
            return None;
        }
        let failure = |message| {
            result::meta_command_failure_result(
                command,
                crate::runtime::room_tool_admission::denied(message),
            )
        };
        let account_request = match tokens {
            [_, verb] if verb == "list-account" => {
                Some(crate::local::AccountKernelsRequest::List {})
            }
            [_, verb, target_id] if verb == "unlink" => {
                Some(crate::local::AccountKernelsRequest::Unlink {
                    target_id: target_id.clone(),
                })
            }
            _ => None,
        };
        if let Some(account_request) = account_request {
            if self.runtime_state.drives_as_room_guest(session, caller) {
                return Some(failure(
                    "Use an agent with the owner's kernel access to manage linked kernels",
                ));
            }
            let request = LocalDaemonRequest::CloudAccountKernels(account_request);
            let command_context = request::meta_kernel_command(provider_run, caller, &request);
            return Some(
                match Box::pin(self.dispatch(command_context, request)).await {
                    Ok(response) => result::meta_command_success_result(
                        command,
                        &response,
                        caller,
                        self.runtime_state.room_agent_tools_enabled(),
                    ),
                    Err(error) => result::meta_command_failure_result(command, error),
                },
            );
        }
        let listing = match tokens {
            [_, verb] if verb == "status" => false,
            [_, verb] if verb == "list-local" => true,
            _ => return Some(failure("use kernel status, kernel list-local, kernel list-account, or kernel unlink <target-id>")),
        };
        if provider_run.is_none() {
            return Some(failure(
                "run this command from an agent on its Room's kernel",
            ));
        }
        if !self
            .runtime_state
            .acts_for_kernel_owner(session.owner_user_id())
            || self.runtime_state.drives_as_room_guest(session, caller)
        {
            return Some(failure(
                "only an agent acting for the kernel owner can read this status",
            ));
        }
        let relay = self.transport_relay_status_snapshot().await;
        let current = self.kernel_identity(&relay);
        let response = if listing {
            match crate::runtime_transport::machine_kernel_presence::list_response(current) {
                Ok(response) => response,
                Err(error) => {
                    return Some(result::meta_command_failure_result(
                        command,
                        result::meta_command_error(format!(
                            "could not read local kernel presence: {error}"
                        )),
                    ));
                }
            }
        } else {
            current.response()
        };
        Some(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"command": command, "response": response}),
        })
    }
}
