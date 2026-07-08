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
mod tests;

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
