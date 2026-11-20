use super::*;

impl CommandRouter {
    pub(super) async fn kernel_identity_command(
        &self,
        local_provider: bool,
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
        let listing = match tokens {
            [_, verb] if verb == "status" => false,
            [_, verb] if verb == "list-local" => true,
            _ => return Some(failure("use kernel status or kernel list-local")),
        };
        if !local_provider {
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
                        format!("could not read local kernel presence: {error}"),
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
