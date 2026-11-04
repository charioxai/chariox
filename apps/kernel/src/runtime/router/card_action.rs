//! Person cards: the router carries out the action of a card the person approved.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::state::APPROVE_CHOICE_ID;
use crate::transport::runtime_tools::{RequestPopupArgs, RuntimeToolResult};

impl CommandRouter {
    /// A `request_popup` call that carries an `action`. The agent waits for the person's
    /// decision; on approval the kernel runs exactly the request the card described and
    /// returns its result. The person's passkey is the authority, not the agent's session.
    pub(super) async fn dispatch_action_card(
        &self,
        run: crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: RequestPopupArgs =
            serde_json::from_value(arguments).map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: format!("invalid tool arguments: {error}"),
            })?;
        if args.view.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: "an action card cannot also carry a browser view".into(),
            });
        }
        let action = args.action.ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_request_popup",
            message: "no action on this card".into(),
        })?;
        let request = crate::runtime::card_action::parse_card_action(action.request)?;
        let mut card = self
            .runtime_state
            .raise_action_card(&run, &request, args.timeout_sec)
            .await?;
        let interaction_id = card.interaction_id.clone();
        let resolution = card.decision().await?;
        if resolution.status != "answered"
            || resolution.choice_id.as_deref() != Some(APPROVE_CHOICE_ID)
        {
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "interaction_id": interaction_id,
                    "status": if resolution.choice_id.as_deref() == Some("cancel") { "cancelled" } else { resolution.status },
                    "choice_id": resolution.choice_id,
                    "ran": false,
                }),
            });
        }
        let mut caller = crate::runtime::command::KernelCaller::for_source(
            &crate::runtime::command::KernelCommandSource::LocalIpc,
        )
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
        caller.caller_id = format!("card-action:{interaction_id}");
        caller.user_id = Some(card.owner_user_id.clone());
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            format!("card-action:{interaction_id}"),
            crate::runtime::command::KernelCommandSource::LocalIpc,
            caller,
            None,
            None,
            &request,
        );
        let payload = match Box::pin(self.dispatch(command, request)).await {
            Ok(response) => serde_json::json!({
                "interaction_id": interaction_id,
                "status": "answered",
                "choice_id": APPROVE_CHOICE_ID,
                "ran": true,
                "response": response,
            }),
            Err(error) => serde_json::json!({
                "interaction_id": interaction_id,
                "status": "answered",
                "choice_id": APPROVE_CHOICE_ID,
                "ran": false,
                "error": crate::secret_redaction::redact_secrets(&error.to_string()),
            }),
        };
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
