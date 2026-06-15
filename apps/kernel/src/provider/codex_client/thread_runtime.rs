use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderWriteAccessMode};

use super::mcp_config::{append_codex_mcp_overrides, append_runtime_mcp_overrides};
use super::permission::codex_permission_policy;
use super::permission::CodexPermissionPolicy;
use super::{CodexClient, CodexNotification, CodexSocket};

#[derive(Debug, Clone, Deserialize)]
pub struct CodexThreadStartResponse {
    pub thread: CodexThread,
    pub model: String,
    #[serde(rename = "reasoningEffort", default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexThread {
    pub id: String,
}

impl CodexClient {
    pub fn thread_start(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        cwd: Option<&str>,
        model: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        developer_instructions: Option<&str>,
    ) -> Result<CodexThreadStartResponse, DaemonError> {
        let params = self.thread_start_params(
            cwd,
            model,
            write_access_mode,
            execution_mode,
            permission_level,
            developer_instructions,
        )?;
        self.send_request(socket, next_request_id, "thread/start", params)
    }

    pub(super) fn thread_start_params(
        &self,
        cwd: Option<&str>,
        model: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        developer_instructions: Option<&str>,
    ) -> Result<Value, DaemonError> {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex thread/start policy",
            json!({
                "provider_run_id": self.provider_run_id,
                "write_access_mode": format!("{write_access_mode:?}"),
                "execution_mode": format!("{execution_mode:?}"),
                "permission_level": format!("{permission_level:?}"),
                "approval_policy": policy.approval_policy,
                "sandbox": policy.sandbox,
                "cwd": cwd,
                "model": model,
            }),
        );
        let mut params = json!({
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "sandbox": policy.sandbox,
            "personality": "pragmatic",
            "persistExtendedHistory": true,
            "serviceName": "arroba",
        });
        let config_overrides = self.thread_config_overrides(&policy)?;
        if !config_overrides.is_empty() {
            self.log_thread_config_overrides("thread/start", &config_overrides);
            params["config"] = json!(config_overrides);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(developer_instructions) = developer_instructions
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params["developerInstructions"] = json!(developer_instructions);
        }
        Ok(params)
    }

    pub fn thread_resume(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        developer_instructions: Option<&str>,
    ) -> Result<CodexThreadStartResponse, DaemonError> {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex thread/resume policy",
            json!({
                "provider_run_id": self.provider_run_id,
                "write_access_mode": format!("{write_access_mode:?}"),
                "execution_mode": format!("{execution_mode:?}"),
                "permission_level": format!("{permission_level:?}"),
                "approval_policy": policy.approval_policy,
                "sandbox": policy.sandbox,
                "cwd": cwd,
                "model": model,
            }),
        );
        let mut params = json!({
            "threadId": thread_id,
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "sandbox": policy.sandbox,
            "personality": "pragmatic",
            "persistExtendedHistory": true,
        });
        let config_overrides = self.thread_config_overrides(&policy)?;
        if !config_overrides.is_empty() {
            self.log_thread_config_overrides("thread/resume", &config_overrides);
            params["config"] = json!(config_overrides);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(developer_instructions) = developer_instructions
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params["developerInstructions"] = json!(developer_instructions);
        }
        self.send_request(socket, next_request_id, "thread/resume", params)
    }

    pub fn turn_start(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        developer_instructions: Option<&str>,
        input: Vec<Value>,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<Value, DaemonError> {
        let params = Self::turn_start_params(
            thread_id,
            input,
            cwd,
            model,
            effort,
            write_access_mode,
            execution_mode,
            permission_level,
            developer_instructions,
        );
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            "turn/start",
            params,
            buffered_notifications,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn turn_start_params(
        thread_id: &str,
        input: Vec<Value>,
        cwd: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        _developer_instructions: Option<&str>,
    ) -> Value {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "personality": "pragmatic",
            "sandboxPolicy": policy.sandbox_policy,
            "summary": "detailed",
        });
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        params
    }

    pub fn turn_interrupt(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), DaemonError> {
        let _: Value = self.send_request(
            socket,
            next_request_id,
            "turn/interrupt",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
            }),
        )?;
        Ok(())
    }

    pub fn thread_turns_list(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<Value, DaemonError> {
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            "thread/turns/list",
            json!({ "threadId": thread_id }),
            buffered_notifications,
        )
    }

    pub(super) fn thread_config_overrides(
        &self,
        policy: &CodexPermissionPolicy,
    ) -> Result<BTreeMap<String, Value>, DaemonError> {
        let mut overrides = policy.config_overrides.clone();
        let provider_mcp_servers = crate::provider::mcp_proxy::provider_facing_mcp_proxy_configs(
            &self.mcp_servers,
            self.runtime_mcp_server_url.as_deref(),
            self.runtime_mcp_auth_token.as_deref(),
        )?;
        append_codex_mcp_overrides(&mut overrides, &provider_mcp_servers);
        if let (Some(server_url), Some(auth_token)) = (
            self.runtime_mcp_server_url.as_deref(),
            self.runtime_mcp_auth_token.as_deref(),
        ) {
            append_runtime_mcp_overrides(&mut overrides, server_url, auth_token);
        }
        Ok(overrides)
    }

    fn log_thread_config_overrides(
        &self,
        operation: &'static str,
        overrides: &BTreeMap<String, Value>,
    ) {
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "sending codex thread config overrides",
            json!({
                "provider_run_id": self.provider_run_id,
                "operation": operation,
                "runtime_mcp_binding_present": self.runtime_mcp_server_url.is_some()
                    && self.runtime_mcp_auth_token.is_some(),
                "granted_mcp_servers": self
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>(),
                "config_override_keys": overrides.keys().cloned().collect::<Vec<_>>(),
            }),
        );
    }
}
