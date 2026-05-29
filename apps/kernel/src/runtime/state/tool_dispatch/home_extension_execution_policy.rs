use super::*;

const HOME_EXTENSION_MAX_RESULT_BYTES: usize = 1024 * 1024;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn cancel_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<bool, DaemonError> {
        super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
            .authorize_invocation_context(context)?;
        let key = remote_extension_invocation_key(metadata);
        let in_flight = self
            .owned
            .remote_extension_invocations
            .lock()
            .await
            .get(&key)
            .is_some_and(Option::is_none);
        if in_flight {
            self.owned
                .remote_extension_cancellations
                .lock()
                .await
                .insert(key);
        }
        self.owned.durable_state_store.append_event(
            "home_extension.invoke.cancelled",
            Some(context.home_agent_id.clone()),
            serde_json::json!({
                "home_session_id": context.home_session_id,
                "home_agent_id": context.home_agent_id,
                "leased_agent_id": context.leased_agent_id,
                "worker_provider_run_id": context.worker_provider_run_id,
                "worker_kernel_id": context.worker_kernel_id,
                "worker_machine_id": context.worker_machine_id,
                "invocation": metadata,
                "status": if in_flight { "cancelled" } else { "not_in_flight" },
            }),
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
        self.owned.durable_state_store.append_event(
            kind,
            Some(context.home_agent_id.clone()),
            serde_json::json!({
                "home_session_id": context.home_session_id,
                "home_agent_id": context.home_agent_id,
                "leased_agent_id": context.leased_agent_id,
                "worker_provider_run_id": context.worker_provider_run_id,
                "worker_kernel_id": context.worker_kernel_id,
                "worker_machine_id": context.worker_machine_id,
                "invocation": metadata,
                "tool": {
                    "kind": tool.kind.as_str(),
                    "name": tool.name,
                    "tool_name": tool.tool_name,
                    "safety": tool.safety,
                    "timeout_sec": tool.timeout_sec,
                    "version_hash": tool.version_hash,
                },
                "status": status,
                "error": error,
            }),
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
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let key = metadata
            .idempotency_key
            .as_deref()
            .unwrap_or(metadata.invocation_id.as_str())
            .to_string();
        if self
            .owned
            .remote_extension_cancellations
            .lock()
            .await
            .contains(&key)
        {
            return Err(home_extension_cancelled_error(metadata));
        }
        let mut invocations = self.owned.remote_extension_invocations.lock().await;
        if let Some(existing) = invocations.get(&key) {
            if let Some(cached) = existing {
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
        invocations.insert(key, None);
        Ok(None)
    }

    pub(in crate::runtime::state::tool_dispatch) async fn complete_home_extension_invocation(
        &self,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        result: serde_json::Value,
    ) -> bool {
        let key = remote_extension_invocation_key(metadata);
        if self
            .owned
            .remote_extension_cancellations
            .lock()
            .await
            .remove(&key)
        {
            self.owned
                .remote_extension_invocations
                .lock()
                .await
                .remove(&key);
            return false;
        }
        self.owned
            .remote_extension_invocations
            .lock()
            .await
            .insert(key, Some(result));
        true
    }

    pub(in crate::runtime::state::tool_dispatch) async fn forget_home_extension_invocation(
        &self,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) {
        let key = remote_extension_invocation_key(metadata);
        self.owned
            .remote_extension_invocations
            .lock()
            .await
            .remove(&key);
        self.owned
            .remote_extension_cancellations
            .lock()
            .await
            .remove(&key);
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
        self.owned.durable_state_store.append_event(
            "home_extension.invoke.completed",
            Some(context.home_agent_id.clone()),
            serde_json::json!({
                "executor": executor,
                "home_session_id": context.home_session_id,
                "home_agent_id": context.home_agent_id,
                "leased_agent_id": context.leased_agent_id,
                "worker_provider_run_id": context.worker_provider_run_id,
                "worker_kernel_id": context.worker_kernel_id,
                "worker_machine_id": context.worker_machine_id,
                "invocation": metadata,
                "tool": {
                    "kind": tool.kind.as_str(),
                    "name": tool.name,
                    "tool_name": tool.tool_name,
                    "safety": tool.safety,
                },
                "status": status,
                "ok": ok,
                "result_bytes": result_bytes,
                "error": error,
            }),
        )?;
        Ok(())
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

fn remote_extension_invocation_key(
    metadata: &crate::extension::RemoteExtensionInvocationMetadata,
) -> String {
    metadata
        .idempotency_key
        .as_deref()
        .unwrap_or(metadata.invocation_id.as_str())
        .to_string()
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
