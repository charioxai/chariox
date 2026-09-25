use chariox_relay::protocol::ClientTarget;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::*;

#[cfg(test)]
type CapabilityResponsePause = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);

#[cfg(test)]
static CAPABILITY_RESPONSE_PAUSES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<usize, CapabilityResponsePause>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(test)]
static CAPABILITY_PUSH_LOCK_OBSERVERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<usize, tokio::sync::oneshot::Sender<bool>>>,
> = std::sync::LazyLock::new(Default::default);

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn pause_capability_response_before_apply_for_test(
        &self,
        entered: tokio::sync::oneshot::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) {
        CAPABILITY_RESPONSE_PAUSES.lock().unwrap().insert(
            std::sync::Arc::as_ptr(&self.app) as usize,
            (entered, release),
        );
    }

    #[cfg(test)]
    pub(crate) fn clear_capability_response_pause_for_test(&self) {
        CAPABILITY_RESPONSE_PAUSES
            .lock()
            .unwrap()
            .remove(&(std::sync::Arc::as_ptr(&self.app) as usize));
        CAPABILITY_PUSH_LOCK_OBSERVERS
            .lock()
            .unwrap()
            .remove(&(std::sync::Arc::as_ptr(&self.app) as usize));
    }

    #[cfg(test)]
    pub(crate) fn observe_capability_push_lock_for_test(
        &self,
        observer: tokio::sync::oneshot::Sender<bool>,
    ) {
        CAPABILITY_PUSH_LOCK_OBSERVERS
            .lock()
            .unwrap()
            .insert(std::sync::Arc::as_ptr(&self.app) as usize, observer);
    }

    #[cfg(test)]
    pub(in crate::runtime::state) fn take_capability_push_lock_observer_for_test(
        &self,
    ) -> Option<tokio::sync::oneshot::Sender<bool>> {
        CAPABILITY_PUSH_LOCK_OBSERVERS
            .lock()
            .unwrap()
            .remove(&(std::sync::Arc::as_ptr(&self.app) as usize))
    }

    #[cfg(test)]
    fn pause_capability_response_before_apply(&self) {
        let pause = CAPABILITY_RESPONSE_PAUSES
            .lock()
            .unwrap()
            .remove(&(std::sync::Arc::as_ptr(&self.app) as usize));
        if let Some((entered, release)) = pause {
            let _ = entered.send(());
            match release.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    panic!("capability response application barrier timed out");
                }
            }
        }
    }

    pub(super) async fn try_dispatch_remote_meta_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .workspace_live_sync_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workspace_live_sync_context_for_provider_run(
                        provider_run.id(),
                        workspace_context.identity.clone(),
                    )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardMetaRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::MetaRuntimeToolHandled { result } => Ok(Some(result)),
            other => Err(DaemonError::LocalTransport {
                operation: "forward leased metaagent runtime tool",
                message: format!("unexpected forwarded metaagent response: {other:?}"),
            }),
        }
    }

    pub(crate) async fn dispatch_forwarded_meta_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let Some(remote) = agent.remote_execution() else {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch forwarded metaagent runtime tool",
                message: format!(
                    "home agent `{}` is not remote-backed",
                    context.home_agent_id
                ),
            });
        };
        if agent.session_id() != context.home_session_id
            || remote.leased_agent_id != context.leased_agent_id
            || remote.worker_kernel_id != context.worker_kernel_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch forwarded metaagent runtime tool",
                message: "forwarded metaagent context does not match the home remote binding"
                    .to_string(),
            });
        }
        self.dispatch_meta_runtime_tool_call_for_agent(
            &context.home_session_id,
            &context.home_agent_id,
            &tool_name,
            arguments,
        )
        .await
    }

    pub(super) async fn try_dispatch_remote_capability_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .workspace_live_sync_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workspace_live_sync_context_for_provider_run(
                        provider_run.id(),
                        workspace_context.identity.clone(),
                    )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(workspace_live_sync_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let mut forwarded_arguments = arguments.clone();
        if crate::transport::runtime_tools::canonical_agent_messaging_tool_name(tool_name)
            == Some(crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL)
        {
            let worker_origin = arguments
                .get("origin_prompt_id")
                .and_then(serde_json::Value::as_str);
            let origin_is_current = match provider_run.agent_instance_id().zip(worker_origin) {
                Some((agent_id, prompt_id)) => self.agent_message_sender_prompt_is_running(
                    provider_run.session_id(),
                    agent_id,
                    prompt_id,
                )?,
                None => false,
            };
            let Some(home_prompt_id) = remote_context.home_prompt_id.as_deref() else {
                return Ok(Some(agent_message_origin_rejected()));
            };
            if !origin_is_current {
                return Ok(Some(agent_message_origin_rejected()));
            }
            // The worker and home allocate different prompt IDs. Validate the
            // worker's turn before translating it to the leased home turn.
            forwarded_arguments["origin_prompt_id"] = serde_json::json!(home_prompt_id);
        }
        let config = self.config_snapshot().await;
        let mut original_response = None;
        let refresh = async {
        for attempt in 0..3 {
            let revision = self
                .owned
                .provider_store
                .get_run(provider_run.id())?
                .remote_extension_manifest_revision();
            // Home may call back into this worker to install a grant. Do not
            // hold the worker app mutex across that outbound request.
            let response =
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(remote_context.home_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::ForwardCapabilityRuntimeTool {
                        context: remote_context.clone(),
                        tool_name: if attempt == 0 {
                            tool_name.to_string()
                        } else {
                            crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL.to_string()
                        },
                        arguments: if attempt == 0 {
                            forwarded_arguments.clone()
                        } else {
                            serde_json::json!({})
                        },
                    },
                )
                .await?;
            let RelayPeerResponse::CapabilityRuntimeToolHandled {
                result,
                skill_package,
                remote_extension_manifest,
            } = response
            else {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased capability runtime tool",
                    message: format!("unexpected forwarded capability response: {response:?}"),
                });
            };
            if original_response.is_none() {
                original_response = Some((result, skill_package));
            }
            #[cfg(test)]
            self.pause_capability_response_before_apply();
            let applied = self
                .with_app_side_effect(|_| {
                    let updated = self
                        .owned
                        .provider_store
                        .compare_update_run_remote_extension_manifest(
                            provider_run.id(),
                            revision,
                            remote_extension_manifest,
                        )?;
                    if let Some(updated) = updated {
                        self.owned.provider_run_projection.update(updated);
                        Ok::<_, DaemonError>(true)
                    } else {
                        Ok(false)
                    }
                })
                .await?;
            if applied {
                break;
            }
            // An intervening push invalidates the response snapshot, even if
            // its contents later become equal again. Refresh with a read-only
            // request; never replay the original tool's side effects.
            if attempt == 2 {
                return Err(DaemonError::LocalTransport {
                    operation: "refresh forwarded capability manifest",
                    message: "original tool completed, but concurrent manifest updates prevented refresh; original tool was not replayed".to_string(),
                });
            }
        }
        Ok::<_, DaemonError>(())
        }.await;
        let Some((mut result, skill_package)) = original_response else {
            return Err(refresh.expect_err("missing response requires a forwarding error"));
        };
        if let Err(error) = refresh {
            // The original operation has already returned its result. A failed
            // read-only refresh must not turn that into an invitation to replay
            // a grant, registration, or message that already happened.
            let warning = serde_json::json!({
                "error": error.to_string(),
                "original_tool_replayed": false,
                "original_result_preserved": true,
            });
            if let Some(payload) = result.payload.as_object_mut() {
                payload.insert("manifest_refresh_warning".to_string(), warning);
            } else {
                result.payload = serde_json::json!({
                    "original_payload": result.payload,
                    "manifest_refresh_warning": warning,
                });
            }
        }
        let result = self.apply_remote_skill_package_response(
            &workspace_context.root,
            &remote_context.home_kernel_id,
            result,
            skill_package,
        )?;
        Ok(Some(result))
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::CharioxSkillPackage>,
            crate::extension::RemoteExtensionManifest,
        ),
        DaemonError,
    > {
        if crate::transport::runtime_tools::canonical_agent_messaging_tool_name(&tool_name)
            == Some(crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL)
        {
            let session = self
                .owned
                .session_store
                .get_session(&context.home_session_id)?;
            let sender = self.owned.agent_store.get_agent(&context.home_agent_id)?;
            let origin_is_current = sender.session_id() == context.home_session_id
                && self.owned.config_projection.snapshot().daemon_id == context.home_kernel_id
                && sender.remote_execution().is_some_and(|binding| {
                    binding.leased_agent_id == context.leased_agent_id
                        && binding.worker_kernel_id == context.worker_kernel_id
                        && binding.worker_machine_id == context.worker_machine_id
                        && binding.active_worker_provider_run_id.as_deref()
                            == Some(context.worker_provider_run_id.as_str())
                })
                && context
                    .home_prompt_id
                    .as_deref()
                    .is_some_and(|home_prompt_id| {
                        self.owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, sender.id())
                            .is_some_and(|prompt| {
                                prompt.id() == home_prompt_id
                                    && prompt.status() == crate::session::PromptStatus::Running
                            })
                    });
            if !origin_is_current {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch forwarded agent message",
                    message: "sender prompt or leased worker binding is no longer current"
                        .to_string(),
                });
            }
        }
        let (result, package) = self
            .dispatch_capability_runtime_tool_call_for_agent(
                &context.home_session_id,
                &context.home_agent_id,
                &tool_name,
                arguments,
                true,
            )
            .await?;
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let manifest = self.remote_extension_manifest_for_agent(&agent)?;
        Ok((result, package, manifest))
    }

    pub(super) async fn dispatch_capability_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "capability tools require an agent-scoped provider run"
                }),
            });
        };
        let session_id = provider_run.session_id().to_string();
        let (result, _) = self
            .dispatch_capability_runtime_tool_call_for_agent(
                &session_id,
                &agent_id,
                tool_name,
                arguments,
                false,
            )
            .await?;
        Ok(result)
    }

    async fn dispatch_capability_runtime_tool_call_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        include_skill_package: bool,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::CharioxSkillPackage>,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        match tool_name {
            crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL => Ok((
                self.handle_list_session_agents_runtime_tool(&session, &agent),
                None,
            )),
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL => Ok((
                self.handle_get_session_agent_runtime_tool(&session, &agent, arguments),
                None,
            )),
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL => Ok((
                self.handle_send_agent_message_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL => {
                self.handle_list_extensions_runtime_tool(&session, &agent, arguments)
            }
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL => {
                self.handle_request_extension_runtime_tool(
                    &session,
                    &agent,
                    session_id,
                    arguments,
                    include_skill_package,
                )
                .await
            }
            crate::transport::runtime_tools::REGISTER_MCP_TOOL => Ok((
                self.handle_register_mcp_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::REGISTER_SKILL_PATH_TOOL => Ok((
                self.handle_register_skill_path_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::REGISTER_ENVIRONMENT_TOOL => Ok((
                self.handle_register_environment_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::REGISTER_SCRIPT_PATH_TOOL => Ok((
                self.handle_register_script_path_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::REGISTER_CONNECTOR_PATH_TOOL => Ok((
                self.handle_register_connector_path_runtime_tool(&session, &agent, arguments)
                    .await?,
                None,
            )),
            crate::transport::runtime_tools::REGISTER_CONNECTOR_ADAPTER_PATH_TOOL => Ok((
                self.handle_register_connector_adapter_path_runtime_tool(
                    &session, &agent, arguments,
                )
                .await?,
                None,
            )),
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_capability_runtime_tool_call",
                message: format!("unknown capability runtime tool `{tool_name}`"),
            }),
        }
    }
}

fn agent_message_origin_rejected() -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "error": "agent message belongs to a different sender turn; message was not sent"
        }),
    }
}
