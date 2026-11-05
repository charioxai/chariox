//! MP-08 / MP-09 / MP-10 / MP-11: both question modes share validation and cards.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

fn popup_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_request_popup",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    pub(super) async fn dispatch_popup_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<crate::transport::runtime_tools::RequestPopupArgs>(
            arguments.clone(),
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_request_popup",
            message: format!("invalid tool arguments: {error}"),
        })?;
        if args.wait == Some(false) {
            let current = self
                .owned
                .provider_store
                .get_run_for_agent(
                    provider_run.session_id(),
                    provider_run
                        .agent_instance_id()
                        .ok_or_else(|| popup_error("provider run has no agent"))?,
                )
                .ok_or_else(|| popup_error("asking provider run is unavailable"))?;
            if current.id() != provider_run.id() {
                return Err(popup_error("stale provider tool authority"));
            }
            if let Some(result) = self
                .try_dispatch_remote_meta_runtime_tool_call(
                    provider_run,
                    crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
        }
        self.dispatch_popup_for_agent(
            provider_run.session_id(),
            provider_run
                .agent_instance_id()
                .ok_or_else(|| popup_error("provider run is not bound to an agent"))?,
            Some(provider_run),
            args,
        )
        .await
    }

    pub(in crate::runtime::state) async fn dispatch_popup_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        args: crate::transport::runtime_tools::RequestPopupArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if args.timeout_sec == Some(0) {
            return Err(popup_error("timeout_sec must be at least 1"));
        }
        if args.choices.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: "popup interactions require at least one choice".to_string(),
            });
        }
        let choices = args
            .choices
            .into_iter()
            .map(|choice| {
                crate::session::RuntimeInteractionChoice::new(
                    choice.id,
                    choice.label,
                    choice.reply,
                    choice.style,
                )
            })
            .collect::<Vec<_>>();
        let custom_choice = args.custom_choice.map(|choice| {
            crate::session::RuntimeInteractionCustomChoice::new(
                choice.id,
                choice.label,
                choice.placeholder,
                choice.min_length,
                choice.max_length,
            )
        });
        if let Some(custom_choice) = custom_choice.as_ref() {
            if choices
                .iter()
                .any(|choice| choice.id() == custom_choice.id())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!(
                        "custom_choice id `{}` duplicates a fixed choice",
                        custom_choice.id()
                    ),
                });
            }
            if let Some(max_length) = custom_choice.max_length() {
                if max_length < custom_choice.min_length() {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message:
                            "custom_choice max_length must be greater than or equal to min_length"
                                .to_string(),
                    });
                }
            }
        }
        let default_choice_id = args.default_on_timeout.clone();
        if let Some(default_choice_id) = default_choice_id.as_deref() {
            if !choices
                .iter()
                .any(|choice| choice.id() == default_choice_id)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!(
                        "default_on_timeout choice `{default_choice_id}` is not defined"
                    ),
                });
            }
        }
        let interaction = crate::session::RuntimeInteraction::new(
            format!("interaction-{:032x}", rand::random::<u128>()),
            agent_id,
            crate::session::RuntimeInteractionKind::Choice,
            args.level
                .unwrap_or(crate::session::RuntimeInteractionLevel::Info),
            args.title,
            args.message,
            choices,
            custom_choice,
            args.timeout_sec,
            default_choice_id.clone(),
        );
        if args.wait == Some(false) {
            return self.arm_interaction_wake(session_id, agent_id, interaction);
        }
        let provider_run =
            provider_run.ok_or_else(|| popup_error("forwarded popup must use wait:false"))?;
        let interaction = interaction.with_native_origin(
            self.owned
                .capture_native_interaction_origin(session_id, agent_id, provider_run.id()),
        );
        let interaction_id = interaction.id().to_string();
        let session_id = provider_run.session_id().to_string();
        let timeout_sec = interaction.timeout_sec();
        let remote_target = if let Some(origin) = interaction.native_origin() {
            self.remote_native_interaction_context(
                provider_run.session_id(),
                provider_run.agent_instance_id().unwrap_or(""),
                origin,
            )
            .await?
        } else {
            None
        };
        if let Some((config, target_daemon_id, context)) = remote_target {
            // Home owns the popup deadline and its resolution. Leave time
            // for that outcome to return; an indefinite card has no relay cap.
            let response_timeout = timeout_sec.map(|seconds| {
                        std::time::Duration::from_secs(seconds)
                            .saturating_add(crate::runtime::native_interaction_bridge::REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER)
                    });
            let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_optional_timeout(
                        &config,
                        ClientTarget { daemon_id: Some(target_daemon_id), daemon_alias: None },
                        RelayPeerRequest::ForwardNativeTurnInteraction { context, interaction },
                        response_timeout,
                    ).await?;
            let resolution = match response {
                RelayPeerResponse::NativeInteractionResolved { resolution } => resolution,
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message: format!(
                            "unexpected relay response for remote popup interaction: {other:?}"
                        ),
                    });
                }
            };
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "interaction_id": interaction_id,
                    "status": resolution.status,
                    "choice_id": resolution.choice_id,
                    "reply": resolution.reply,
                }),
            });
        }
        let resolution_rx = self
            .create_runtime_interaction(&session_id, interaction)
            .await?;
        if let Some(timeout_sec) = timeout_sec {
            let state = self.clone();
            let timeout_session_id = session_id.clone();
            let timeout_interaction_id = interaction_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        let resolution = resolution_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: format!("popup interaction dropped before resolution: {error}"),
            })?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "interaction_id": interaction_id,
                "status": resolution.status,
                "choice_id": resolution.choice_id,
                "reply": resolution.reply,
            }),
        })
    }
}
