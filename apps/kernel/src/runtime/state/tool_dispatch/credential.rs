use arroba_relay::protocol::ClientTarget;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

impl KernelRuntimeState {
    pub(super) async fn dispatch_credential_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let service = crate::secret::RuntimeSecretService::with_vault_service(
            user_config.credentials,
            user_config.credential_vault.service,
        );
        match tool_name {
            crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL => {
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credentials": service.list_handles()
                    }),
                })
            }
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::HttpRequestWithCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let request = crate::secret::CredentialHttpRequest {
                    credential_id: args.credential_id,
                    method: args.method,
                    url: args.url,
                    headers: args.headers,
                    body_text: args.body_text,
                    body_json: args.body_json,
                };
                let response = tokio::task::spawn_blocking(move || {
                    service.http_request_with_credential(request)
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: error.to_string(),
                })??;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(response).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_http_request_with_credential",
                            message: error.to_string(),
                        }
                    })?,
                })
            }
            crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SendSecretToTerminalArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_send_secret_to_terminal",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut input = service.terminal_secret_input(&args.credential_id)?;
                if args.append_newline {
                    input.push('\n');
                }
                let provider_run_id = provider_run.id().to_string();
                self.with_app_side_effect(move |app| {
                    app.write_provider_pty_input_for_runtime(&provider_run_id, input.as_bytes())
                })
                .await?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "credential_id": args.credential_id,
                        "target": "current_provider_run",
                    }),
                })
            }
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::RequestPopupArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                if args.choices.len() < 2 {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message: "popup interactions require at least two choices".to_string(),
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
                                message: "custom_choice max_length must be greater than or equal to min_length".to_string(),
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
                    format!(
                        "interaction-{}-{}",
                        provider_run.agent_instance_id().unwrap_or("agent"),
                        crate::session::unix_epoch_ms()
                    ),
                    provider_run.agent_instance_id().ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: "provider run is not bound to an agent".to_string(),
                        }
                    })?,
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
                let interaction_id = interaction.id().to_string();
                let session_id = provider_run.session_id().to_string();
                let timeout_sec = interaction.timeout_sec();
                let remote_target = self
                    .with_app_side_effect(|app| {
                        let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                        runtime.native_interaction_context_for_backing_agent(
                            provider_run.session_id(),
                            provider_run.agent_instance_id().unwrap_or(""),
                            provider_run.id(),
                        )
                    })
                    .await;
                if let Some((target_daemon_id, context)) = remote_target {
                    let response = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(target_daemon_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::ForwardNativeInteraction {
                                        context: context.clone(),
                                        interaction: interaction.clone(),
                                    },
                                ),
                            )
                        })
                        .await?;
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
                            .timeout_runtime_interaction(
                                &timeout_session_id,
                                &timeout_interaction_id,
                            )
                            .await;
                    });
                }
                let resolution =
                    resolution_rx
                        .await
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "runtime_tool_request_popup",
                            message: format!(
                                "popup interaction dropped before resolution: {error}"
                            ),
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
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_credential_runtime_tool_call",
                message: format!("unknown credential runtime tool `{tool_name}`"),
            }),
        }
    }
}
