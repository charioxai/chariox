//! Records each MCP tool call a local agent makes through the kernel's MCP proxy, next to
//! the grant events. It keeps the server, the tool and the outcome and never the arguments
//! or the output; the proxy is the one place every granted MCP server's traffic passes.

use super::*;

pub(in crate::runtime::state) const LOCAL_MCP_TOOL_CALL_EVENT: &str = "agent.mcp_tool_call";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime::state) enum LocalMcpCallStatus {
    Allowed,
    Denied,
    Error,
}

impl LocalMcpCallStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }
}

/// The tool a JSON-RPC request calls, or `None` for every other MCP message.
pub(in crate::runtime::state) fn mcp_tool_call_name(payload: &serde_json::Value) -> Option<String> {
    if payload.get("method")?.as_str()? != "tools/call" {
        return None;
    }
    Some(payload.get("params")?.get("name")?.as_str()?.to_string())
}

/// The outcome of one proxied call, with a fixed reason: server error text can quote the
/// arguments, so it is never copied into the audit.
pub(in crate::runtime::state) fn classify_mcp_tool_call(
    outcome: &Result<serde_json::Value, DaemonError>,
) -> (LocalMcpCallStatus, Option<&'static str>) {
    match outcome {
        Err(DaemonError::LocalTransport {
            operation: "mcp.proxy.grant" | "remote MCP proxy",
            ..
        }) => (
            LocalMcpCallStatus::Denied,
            Some("the server is not granted to this agent"),
        ),
        Err(_) => (
            LocalMcpCallStatus::Error,
            Some("the MCP server could not be reached"),
        ),
        Ok(response) if response.get("error").is_some() => (
            LocalMcpCallStatus::Error,
            Some("the MCP server answered with an error"),
        ),
        Ok(response)
            if response
                .pointer("/result/isError")
                .and_then(serde_json::Value::as_bool)
                == Some(true) =>
        {
            (
                LocalMcpCallStatus::Error,
                Some("the tool reported an error"),
            )
        }
        Ok(_) => (LocalMcpCallStatus::Allowed, None),
    }
}

impl KernelRuntimeState {
    pub(in crate::runtime::state) fn record_local_mcp_tool_call(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        server: &str,
        tool: &str,
        outcome: &Result<serde_json::Value, DaemonError>,
    ) {
        let Some(agent_id) = run.agent_instance_id() else {
            return;
        };
        let (status, reason) = classify_mcp_tool_call(outcome);
        let payload = serde_json::json!({
            "agent_id": agent_id,
            "home_session_id": run.session_id(),
            "provider_run_id": run.id(),
            "tool": { "kind": "mcp", "name": server, "tool_name": tool },
            "status": status.as_str(),
            "error": reason,
        });
        if let Err(error) = self.owned.durable_state_store.append_event(
            LOCAL_MCP_TOOL_CALL_EVENT,
            Some(agent_id.to_string()),
            payload,
        ) {
            crate::logging::error_with_fields(
                "daemon.runtime.mcp_audit",
                "recording a local MCP tool call in the audit failed; the call itself was not affected",
                serde_json::json!({
                    "agent_id": agent_id,
                    "server": server,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tools_call_requests_name_a_tool() {
        let call = serde_json::json!({"method": "tools/call", "params": {"name": "read_page", "arguments": {"q": "x"}}});
        assert_eq!(mcp_tool_call_name(&call).as_deref(), Some("read_page"));
        for other in [
            serde_json::json!({"method": "tools/list"}),
            serde_json::json!({"method": "initialize", "params": {"name": "x"}}),
            serde_json::json!({"method": "tools/call", "params": {}}),
        ] {
            assert_eq!(mcp_tool_call_name(&other), None);
        }
    }

    #[test]
    fn outcomes_become_allowed_denied_or_error_without_server_text() {
        let grant = DaemonError::LocalTransport {
            operation: "mcp.proxy.grant",
            message: "secret-argument".into(),
        };
        let transport = DaemonError::LocalTransport {
            operation: "mcp.proxy.dispatch",
            message: "secret-argument".into(),
        };
        assert_eq!(
            classify_mcp_tool_call(&Err(grant)).0,
            LocalMcpCallStatus::Denied
        );
        assert_eq!(
            classify_mcp_tool_call(&Err(transport)).0,
            LocalMcpCallStatus::Error
        );
        let rpc_error = serde_json::json!({"error": {"message": "secret-argument"}});
        let (status, reason) = classify_mcp_tool_call(&Ok(rpc_error));
        assert_eq!(status, LocalMcpCallStatus::Error);
        assert!(!reason.unwrap_or_default().contains("secret"));
        let tool_error = serde_json::json!({"result": {"isError": true}});
        assert_eq!(
            classify_mcp_tool_call(&Ok(tool_error)).0,
            LocalMcpCallStatus::Error
        );
        let fine = serde_json::json!({"result": {"content": []}});
        assert_eq!(
            classify_mcp_tool_call(&Ok(fine)),
            (LocalMcpCallStatus::Allowed, None)
        );
    }
}
