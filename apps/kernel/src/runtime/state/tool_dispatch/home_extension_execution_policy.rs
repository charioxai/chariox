use super::*;

const HOME_EXTENSION_MAX_RESULT_BYTES: usize = 1024 * 1024;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        hinted_tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let tool =
            match super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
                .authorize_invocation(&context, &hinted_tool)
            {
                Ok(tool) => tool,
                Err(error) => {
                    let _ = self
                        .append_home_extension_denied_event(
                            &context,
                            &metadata,
                            &hinted_tool,
                            &error,
                        )
                        .await;
                    return Err(error);
                }
            };
        if !matches!(
            tool.kind,
            crate::extension::ExtensionKind::Script | crate::extension::ExtensionKind::Connector
        ) {
            let error = DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: format!(
                    "home extension runtime tool invocation only supports scripts and connectors; `{}` is `{}` and must use its dedicated dispatch path",
                    tool.tool_name,
                    tool.kind.as_str()
                ),
            };
            let _ = self
                .append_home_extension_denied_event(&context, &metadata, &tool, &error)
                .await;
            return Err(error);
        }
        if let Some(cached) = self
            .begin_audited_home_extension_invocation(&context, &metadata, &tool)
            .await?
        {
            return serde_json::from_value(cached).map_err(|error| DaemonError::LocalTransport {
                operation: "home extension invocation replay",
                message: error.to_string(),
            });
        }
        self.append_home_extension_audit_event(
            "home_extension.invoke.accepted",
            &context,
            &metadata,
            &tool,
            None,
            None,
        )
        .await?;
        let result = match tool.kind.clone() {
            crate::extension::ExtensionKind::Script => with_home_extension_timeout(
                &tool,
                "home script proxy",
                self.dispatch_home_script_tool(&context, &tool, arguments),
            )
            .await
            .and_then(enforce_home_extension_runtime_result_limit),
            crate::extension::ExtensionKind::Connector => with_home_extension_timeout(
                &tool,
                "home connector proxy",
                self.dispatch_home_connector_tool(&context, &tool, arguments),
            )
            .await
            .and_then(enforce_home_extension_runtime_result_limit),
            _ => Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "home extension runtime tool invocation only supports scripts and connectors"
                }),
            }),
        };
        if let Ok(result) = &result {
            let completed = self
                .complete_home_extension_invocation(
                    &context,
                    &tool,
                    &metadata,
                    serde_json::to_value(result).map_err(|error| DaemonError::LocalTransport {
                        operation: "home extension invocation replay",
                        message: error.to_string(),
                    })?,
                )
                .await;
            if !completed {
                let cancelled = home_extension_cancelled_error(&metadata);
                let audit_cancelled = home_extension_cancelled_error(&metadata);
                self.audit_home_runtime_tool_result(
                    "home-extension",
                    &context,
                    &metadata,
                    &tool,
                    &Err(audit_cancelled),
                )
                .await?;
                return Err(cancelled);
            }
        } else {
            self.forget_home_extension_invocation(&context, &tool, &metadata)
                .await;
        }
        match tool.kind.clone() {
            crate::extension::ExtensionKind::Script => {
                self.audit_home_runtime_tool_result("script", &context, &metadata, &tool, &result)
                    .await?;
                result
            }
            crate::extension::ExtensionKind::Connector => {
                self.audit_home_runtime_tool_result(
                    "connector",
                    &context,
                    &metadata,
                    &tool,
                    &result,
                )
                .await?;
                result
            }
            _ => result,
        }
    }

    pub(crate) async fn cancel_forwarded_home_extension_invocation(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<bool, DaemonError> {
        self.cancel_home_extension_invocation(&context, &metadata)
            .await
    }

    pub(in crate::runtime::state::tool_dispatch) async fn cancel_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<bool, DaemonError> {
        super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
            .authorize_invocation_context(context)?;
        let cancel_key = remote_extension_invocation_cancel_key(context, metadata);
        let in_flight = self
            .owned
            .remote_extension_invocations
            .lock()
            .await
            .iter()
            .any(|(key, value)| {
                home_extension_invocation_matches_cancel(key, value, context, metadata)
            });
        if in_flight {
            self.owned
                .remote_extension_cancellations
                .lock()
                .await
                .insert(cancel_key);
        }
        self.owned.durable_state_store.append_event(
            "home_extension.invoke.cancelled",
            Some(context.home_agent_id.clone()),
            {
                let mut payload = self.home_extension_invocation_audit_payload(context, metadata);
                payload.insert("invocation".to_string(), serde_json::json!(metadata));
                payload.insert(
                    "status".to_string(),
                    serde_json::json!(if in_flight {
                        "cancelled"
                    } else {
                        "not_in_flight"
                    }),
                );
                serde_json::Value::Object(payload)
            },
        )?;
        Ok(in_flight)
    }

    pub(in crate::runtime::state::tool_dispatch) async fn append_home_extension_audit_event(
        &self,
        kind: &'static str,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
        status: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_invocation_audit_payload(context, metadata);
        payload.insert("invocation".to_string(), serde_json::json!(metadata));
        payload.insert(
            "tool".to_string(),
            serde_json::json!({
                "kind": tool.kind.as_str(),
                "name": tool.name,
                "tool_name": tool.tool_name,
                "safety": tool.safety,
                "timeout_sec": tool.timeout_sec,
                "version_hash": tool.version_hash,
            }),
        );
        payload.insert("status".to_string(), serde_json::json!(status));
        payload.insert("error".to_string(), serde_json::json!(error));
        self.owned.durable_state_store.append_event(
            kind,
            Some(context.home_agent_id.clone()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    pub(in crate::runtime::state::tool_dispatch) async fn append_home_extension_denied_event(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        hinted_tool: &crate::extension::RemoteExtensionTool,
        error: &DaemonError,
    ) -> Result<(), DaemonError> {
        self.append_home_extension_audit_event(
            "home_extension.invoke.denied",
            context,
            metadata,
            hinted_tool,
            Some("denied"),
            Some(&error.to_string()),
        )
        .await
    }

    pub(in crate::runtime::state::tool_dispatch) async fn begin_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let key = remote_extension_invocation_key(context, tool, metadata);
        let cancel_key = remote_extension_invocation_cancel_key(context, metadata);
        let cancellations = self.owned.remote_extension_cancellations.lock().await;
        if cancellations.contains(&cancel_key) {
            return Err(home_extension_cancelled_error(metadata));
        }
        drop(cancellations);
        let mut invocations = self.owned.remote_extension_invocations.lock().await;
        if let Some(existing) = invocations.get(&key) {
            if let Some(cached) = &existing.result {
                if metadata.idempotency_key.is_some() {
                    return Ok(Some(cached.clone()));
                }
                return Err(DaemonError::LocalTransport {
                    operation: "home extension invocation replay",
                    message: format!(
                        "duplicate non-idempotent home extension invocation `{}`",
                        metadata.invocation_id
                    ),
                });
            }
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation replay",
                message: format!(
                    "home extension invocation `{}` is already in progress",
                    metadata.invocation_id
                ),
            });
        }
        invocations.insert(
            key,
            RemoteExtensionInvocationState {
                invocation_id: metadata.invocation_id.clone(),
                result: None,
            },
        );
        Ok(None)
    }

    pub(in crate::runtime::state::tool_dispatch) async fn begin_audited_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        match self
            .begin_home_extension_invocation(context, tool, metadata)
            .await
        {
            Ok(Some(cached)) => {
                self.append_home_extension_audit_event(
                    "home_extension.invoke.replayed",
                    context,
                    metadata,
                    tool,
                    Some("replayed"),
                    None,
                )
                .await?;
                Ok(Some(cached))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                let _ = self
                    .append_home_extension_denied_event(context, metadata, tool, &error)
                    .await;
                Err(error)
            }
        }
    }

    pub(in crate::runtime::state::tool_dispatch) async fn complete_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        result: serde_json::Value,
    ) -> bool {
        let key = remote_extension_invocation_key(context, tool, metadata);
        let cancel_key = remote_extension_invocation_cancel_key(context, metadata);
        let mut cancellations = self.owned.remote_extension_cancellations.lock().await;
        let cancelled = cancellations.remove(&cancel_key);
        drop(cancellations);
        if cancelled {
            self.owned
                .remote_extension_invocations
                .lock()
                .await
                .remove(&key);
            return false;
        }
        self.owned.remote_extension_invocations.lock().await.insert(
            key,
            RemoteExtensionInvocationState {
                invocation_id: metadata.invocation_id.clone(),
                result: Some(result),
            },
        );
        true
    }

    pub(in crate::runtime::state::tool_dispatch) async fn forget_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) {
        let key = remote_extension_invocation_key(context, tool, metadata);
        let cancel_key = remote_extension_invocation_cancel_key(context, metadata);
        self.owned
            .remote_extension_invocations
            .lock()
            .await
            .remove(&key);
        self.owned
            .remote_extension_cancellations
            .lock()
            .await
            .remove(&cancel_key);
    }

    pub(in crate::runtime::state::tool_dispatch) async fn audit_home_runtime_tool_result(
        &self,
        executor: &'static str,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
        result: &Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError>,
    ) -> Result<(), DaemonError> {
        let (status, ok, result_bytes, error) = match result {
            Ok(result) => (
                "completed",
                Some(result.ok),
                serde_json::to_vec(&result.payload)
                    .ok()
                    .map(|bytes| bytes.len()),
                None,
            ),
            Err(error) => ("failed", None, None, Some(error.to_string())),
        };
        let mut payload = self.home_extension_invocation_audit_payload(context, metadata);
        payload.insert("executor".to_string(), serde_json::json!(executor));
        payload.insert("invocation".to_string(), serde_json::json!(metadata));
        payload.insert(
            "tool".to_string(),
            serde_json::json!({
                "kind": tool.kind.as_str(),
                "name": tool.name,
                "tool_name": tool.tool_name,
                "safety": tool.safety,
                "timeout_sec": tool.timeout_sec,
                "version_hash": tool.version_hash,
            }),
        );
        payload.insert("status".to_string(), serde_json::json!(status));
        payload.insert("ok".to_string(), serde_json::json!(ok));
        payload.insert("result_bytes".to_string(), serde_json::json!(result_bytes));
        payload.insert("error".to_string(), serde_json::json!(error));
        self.owned.durable_state_store.append_event(
            "home_extension.invoke.completed",
            Some(context.home_agent_id.clone()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    pub(in crate::runtime::state::tool_dispatch) fn home_extension_invocation_audit_payload(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> serde_json::Map<String, serde_json::Value> {
        let agent = self
            .owned
            .agent_store
            .get_agent(&context.home_agent_id)
            .ok();
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)
            .ok();
        let remote_execution = agent.as_ref().and_then(|agent| agent.remote_execution());
        let mut payload = serde_json::Map::new();
        payload.insert(
            "home_session_id".to_string(),
            serde_json::json!(context.home_session_id),
        );
        payload.insert(
            "home_user_id".to_string(),
            serde_json::json!(session.as_ref().map(|session| session.owner_user_id())),
        );
        payload.insert(
            "caller_user_id".to_string(),
            serde_json::json!(agent.as_ref().map(|agent| agent.owner_user_id())),
        );
        payload.insert(
            "agent_id".to_string(),
            serde_json::json!(context.home_agent_id),
        );
        payload.insert(
            "agent_ref".to_string(),
            serde_json::json!(agent.as_ref().map(|agent| agent.agent_ref())),
        );
        payload.insert(
            "agent_owner_user_id".to_string(),
            serde_json::json!(agent.as_ref().map(|agent| agent.owner_user_id())),
        );
        payload.insert(
            "lease_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.execution_lease_id.as_str())),
        );
        payload.insert(
            "leased_agent_id".to_string(),
            serde_json::json!(context.leased_agent_id),
        );
        payload.insert(
            "worker_provider_run_id".to_string(),
            serde_json::json!(context.worker_provider_run_id),
        );
        payload.insert(
            "active_worker_provider_run_id".to_string(),
            serde_json::json!(
                remote_execution.and_then(|remote| remote.active_worker_provider_run_id.as_deref())
            ),
        );
        payload.insert(
            "worker_kernel_id".to_string(),
            serde_json::json!(context.worker_kernel_id),
        );
        payload.insert(
            "worker_machine_id".to_string(),
            serde_json::json!(context.worker_machine_id),
        );
        payload.insert(
            "duration_ms".to_string(),
            serde_json::json!(
                crate::session::unix_epoch_ms().saturating_sub(metadata.started_at_ms)
            ),
        );
        payload
    }
}

pub(in crate::runtime::state::tool_dispatch) fn home_extension_cancelled_error(
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "home extension invocation cancellation",
        message: format!(
            "home extension invocation `{}` was cancelled",
            metadata.invocation_id
        ),
    }
}

fn remote_extension_invocation_context_prefix(
    context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> String {
    let base = remote_extension_invocation_replay_base(metadata);
    format!(
        "home_session={}|home_agent={}|leased_agent={}|worker_run={}|base={}|",
        context.home_session_id,
        context.home_agent_id,
        context.leased_agent_id,
        context.worker_provider_run_id,
        base,
    )
}

fn remote_extension_invocation_replay_base(
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> &str {
    metadata
        .idempotency_key
        .as_deref()
        .or(metadata.provider_tool_call_id.as_deref())
        .unwrap_or(metadata.invocation_id.as_str())
}

fn remote_extension_invocation_cancel_key(
    context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> String {
    format!(
        "home_session={}|home_agent={}|leased_agent={}|worker_run={}|invocation={}",
        context.home_session_id,
        context.home_agent_id,
        context.leased_agent_id,
        context.worker_provider_run_id,
        metadata.invocation_id,
    )
}

fn remote_extension_invocation_key(
    context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    tool: &crate::extension::RemoteExtensionTool,
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> String {
    let suffix = if metadata.idempotency_key.is_some() {
        format!("tool={}", tool.tool_name)
    } else {
        "non_idempotent".to_string()
    };
    format!(
        "{}{}",
        remote_extension_invocation_context_prefix(context, metadata),
        suffix,
    )
}

fn home_extension_invocation_matches_cancel(
    key: &str,
    value: &RemoteExtensionInvocationState,
    context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> bool {
    key.starts_with(&remote_extension_invocation_context_prefix(
        context, metadata,
    )) && value.invocation_id == metadata.invocation_id
        && value.result.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn test_runtime_state() -> KernelRuntimeState {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                .expect("daemon bootstrap should succeed"),
        ));
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(&app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
        )
    }

    fn metadata(invocation_id: &str) -> crate::extension::RemoteExtensionInvocationMetadata {
        crate::extension::RemoteExtensionInvocationMetadata {
            invocation_id: invocation_id.to_string(),
            provider_tool_call_id: None,
            attempt: 1,
            idempotency_key: None,
            started_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    fn metadata_with_provider_call(
        invocation_id: &str,
        provider_tool_call_id: &str,
    ) -> crate::extension::RemoteExtensionInvocationMetadata {
        crate::extension::RemoteExtensionInvocationMetadata {
            invocation_id: invocation_id.to_string(),
            provider_tool_call_id: Some(provider_tool_call_id.to_string()),
            attempt: 1,
            idempotency_key: None,
            started_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    fn context(agent_id: &str) -> crate::transport::relay_peer::RemoteExtensionInvocationContext {
        crate::transport::relay_peer::RemoteExtensionInvocationContext {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "session-1".to_string(),
            home_agent_id: agent_id.to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            worker_provider_run_id: "provider-run-1".to_string(),
            worker_kernel_id: Some("worker-kernel".to_string()),
            worker_machine_id: Some("worker-machine".to_string()),
        }
    }

    fn tool(name: &str) -> crate::extension::RemoteExtensionTool {
        crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Script,
            name: name.to_string(),
            tool_name: name.to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::json!({}),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(30),
            version_hash: Some(format!("{name}-hash")),
        }
    }

    #[tokio::test]
    async fn home_extension_invocation_replays_completed_idempotent_result() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let mut metadata = metadata("invoke-1");
        metadata.idempotency_key = Some("idem-1".to_string());
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("first invocation should start")
            .is_none());
        assert!(
            state
                .complete_home_extension_invocation(
                    &context,
                    &tool,
                    &metadata,
                    serde_json::json!({"ok": true, "value": 42}),
                )
                .await
        );

        let replayed = state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("idempotent duplicate should replay")
            .expect("replay should return cached result");
        assert_eq!(replayed, serde_json::json!({"ok": true, "value": 42}));
    }

    #[tokio::test]
    async fn home_extension_idempotency_replay_is_scoped_to_tool() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let first_tool = tool("lookup");
        let second_tool = tool("create_issue");
        let mut metadata = metadata("invoke-scoped");
        metadata.idempotency_key = Some("shared-idempotency-key".to_string());
        assert!(state
            .begin_home_extension_invocation(&context, &first_tool, &metadata)
            .await
            .expect("first tool invocation should start")
            .is_none());
        assert!(
            state
                .complete_home_extension_invocation(
                    &context,
                    &first_tool,
                    &metadata,
                    serde_json::json!({"tool": "lookup"}),
                )
                .await
        );

        assert!(state
            .begin_home_extension_invocation(&context, &second_tool, &metadata)
            .await
            .expect("same idempotency key on another tool should not replay")
            .is_none());
    }

    #[tokio::test]
    async fn home_extension_idempotency_replay_is_scoped_to_agent() {
        let state = test_runtime_state().await;
        let first_context = context("agent-1");
        let second_context = context("agent-2");
        let tool = tool("lookup");
        let mut metadata = metadata("invoke-agent-scoped");
        metadata.idempotency_key = Some("shared-agent-idempotency-key".to_string());
        assert!(state
            .begin_home_extension_invocation(&first_context, &tool, &metadata)
            .await
            .expect("first agent invocation should start")
            .is_none());
        assert!(
            state
                .complete_home_extension_invocation(
                    &first_context,
                    &tool,
                    &metadata,
                    serde_json::json!({"agent": "agent-1"}),
                )
                .await
        );

        assert!(state
            .begin_home_extension_invocation(&second_context, &tool, &metadata)
            .await
            .expect("same idempotency key on another agent should not replay")
            .is_none());
    }

    #[tokio::test]
    async fn home_extension_invocation_rejects_completed_non_idempotent_duplicate() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let metadata = metadata("invoke-2");
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("first invocation should start")
            .is_none());
        assert!(
            state
                .complete_home_extension_invocation(
                    &context,
                    &tool,
                    &metadata,
                    serde_json::json!({"ok": true}),
                )
                .await
        );

        let error = state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect_err("non-idempotent duplicate should be rejected");
        assert!(
            error
                .to_string()
                .contains("duplicate non-idempotent home extension invocation"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn home_extension_invocation_rejects_duplicate_provider_tool_call_without_idempotency() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let first = metadata_with_provider_call("invoke-provider-1", "provider-call-1");
        let second = metadata_with_provider_call("invoke-provider-2", "provider-call-1");
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &first)
            .await
            .expect("first provider tool call should start")
            .is_none());
        assert!(
            state
                .complete_home_extension_invocation(
                    &context,
                    &tool,
                    &first,
                    serde_json::json!({"ok": true}),
                )
                .await
        );

        let error = state
            .begin_home_extension_invocation(&context, &tool, &second)
            .await
            .expect_err("duplicate provider tool call should be rejected");
        assert!(
            error
                .to_string()
                .contains("duplicate non-idempotent home extension invocation"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn home_extension_invocation_rejects_in_flight_duplicate_provider_tool_call() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let first = metadata_with_provider_call("invoke-provider-inflight-1", "provider-call-2");
        let second = metadata_with_provider_call("invoke-provider-inflight-2", "provider-call-2");
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &first)
            .await
            .expect("first provider tool call should start")
            .is_none());

        let error = state
            .begin_home_extension_invocation(&context, &tool, &second)
            .await
            .expect_err("in-flight duplicate provider tool call should be rejected");
        assert!(
            error.to_string().contains("is already in progress"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn home_extension_invocation_rejects_in_flight_duplicate() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let metadata = metadata("invoke-3");
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("first invocation should start")
            .is_none());

        let error = state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect_err("in-flight duplicate should be rejected");
        assert!(
            error.to_string().contains("is already in progress"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn home_extension_invocation_cancelled_in_flight_is_not_cached_for_replay() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let tool = tool("lookup");
        let mut metadata = metadata("invoke-4");
        metadata.idempotency_key = Some("idem-cancelled".to_string());
        assert!(state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("first invocation should start")
            .is_none());

        state
            .owned
            .remote_extension_cancellations
            .lock()
            .await
            .insert(remote_extension_invocation_cancel_key(&context, &metadata));
        assert!(
            !state
                .complete_home_extension_invocation(
                    &context,
                    &tool,
                    &metadata,
                    serde_json::json!({"ok": true, "value": "late"}),
                )
                .await,
            "late completion after cancellation must be suppressed"
        );

        assert!(state
            .begin_home_extension_invocation(&context, &tool, &metadata)
            .await
            .expect("cancelled invocation should not leave replay state behind")
            .is_none());
    }

    #[tokio::test]
    async fn home_extension_cancellation_is_scoped_to_invocation_not_shared_idempotency_key() {
        let state = test_runtime_state().await;
        let context = context("agent-1");
        let first_tool = tool("lookup");
        let second_tool = tool("create_issue");
        let mut first_metadata = metadata("invoke-cancel-first");
        first_metadata.idempotency_key = Some("shared-cancel-idempotency-key".to_string());
        let mut second_metadata = metadata("invoke-cancel-second");
        second_metadata.idempotency_key = first_metadata.idempotency_key.clone();

        assert!(state
            .begin_home_extension_invocation(&context, &first_tool, &first_metadata)
            .await
            .expect("first invocation should start")
            .is_none());
        assert!(state
            .begin_home_extension_invocation(&context, &second_tool, &second_metadata)
            .await
            .expect("second tool with same idempotency key should start independently")
            .is_none());

        state
            .owned
            .remote_extension_cancellations
            .lock()
            .await
            .insert(remote_extension_invocation_cancel_key(
                &context,
                &first_metadata,
            ));
        assert!(
            !state
                .complete_home_extension_invocation(
                    &context,
                    &first_tool,
                    &first_metadata,
                    serde_json::json!({"tool": "lookup"}),
                )
                .await,
            "cancelled invocation must not cache a replay result"
        );
        assert!(
            state
                .complete_home_extension_invocation(
                    &context,
                    &second_tool,
                    &second_metadata,
                    serde_json::json!({"tool": "create_issue"}),
                )
                .await,
            "cancelling one tool call must not cancel another tool with the same idempotency key"
        );

        let replayed = state
            .begin_home_extension_invocation(&context, &second_tool, &second_metadata)
            .await
            .expect("second invocation replay should be available")
            .expect("second result should be cached");
        assert_eq!(replayed, serde_json::json!({"tool": "create_issue"}));
    }

    #[test]
    fn home_extension_cancel_match_is_scoped_to_context() {
        let first_context = context("agent-1");
        let second_context = context("agent-2");
        let tool = tool("lookup");
        let metadata = metadata("invoke-shared");
        let key = remote_extension_invocation_key(&first_context, &tool, &metadata);
        let value = RemoteExtensionInvocationState {
            invocation_id: metadata.invocation_id.clone(),
            result: None,
        };

        assert!(home_extension_invocation_matches_cancel(
            &key,
            &value,
            &first_context,
            &metadata,
        ));
        assert!(
            !home_extension_invocation_matches_cancel(&key, &value, &second_context, &metadata),
            "cancelling another context with the same invocation id must not mark this call in-flight",
        );
    }

    #[test]
    fn home_extension_runtime_result_limit_rejects_oversized_payload() {
        let result = crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "blob": "x".repeat(HOME_EXTENSION_MAX_RESULT_BYTES),
            }),
        };
        let error = enforce_home_extension_runtime_result_limit(result)
            .expect_err("oversized runtime tool result should be rejected");
        assert!(
            error.to_string().contains("home extension result exceeded"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn home_extension_json_result_limit_rejects_oversized_payload() {
        let error = enforce_home_extension_json_result_limit(serde_json::json!({
            "blob": "x".repeat(HOME_EXTENSION_MAX_RESULT_BYTES),
        }))
        .expect_err("oversized MCP proxy result should be rejected");
        assert!(
            error.to_string().contains("home extension result exceeded"),
            "unexpected error: {error}"
        );
    }
}

pub(in crate::runtime::state::tool_dispatch) fn enforce_home_extension_runtime_result_limit(
    result: crate::transport::runtime_tools::RuntimeToolResult,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    enforce_home_extension_value_size(&result.payload)?;
    Ok(result)
}

pub(in crate::runtime::state::tool_dispatch) fn enforce_home_extension_json_result_limit(
    result: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    enforce_home_extension_value_size(&result)?;
    Ok(result)
}

pub(in crate::runtime::state::tool_dispatch) async fn with_home_extension_timeout<T, Fut>(
    tool: &crate::extension::RemoteExtensionTool,
    operation: &'static str,
    future: Fut,
) -> Result<T, DaemonError>
where
    Fut: std::future::Future<Output = Result<T, DaemonError>>,
{
    let Some(timeout_sec) = tool.timeout_sec.filter(|timeout| *timeout > 0) else {
        return future.await;
    };
    tokio::time::timeout(std::time::Duration::from_secs(timeout_sec), future)
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation,
            message: format!(
                "home extension `{}` timed out after {}s",
                tool.tool_name, timeout_sec
            ),
        })?
}

fn enforce_home_extension_value_size(value: &serde_json::Value) -> Result<(), DaemonError> {
    let size = serde_json::to_vec(value)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "home extension result limit",
            message: error.to_string(),
        })?
        .len();
    if size > HOME_EXTENSION_MAX_RESULT_BYTES {
        return Err(DaemonError::LocalTransport {
            operation: "home extension result limit",
            message: format!(
                "home extension result exceeded {} bytes ({size} bytes)",
                HOME_EXTENSION_MAX_RESULT_BYTES
            ),
        });
    }
    Ok(())
}
